// rtcl-worker.js — rtcl 的 worker 部署形态（网页 dispatcher 的真实运行
// 结构）：解释器 + JS 桥全部在本 worker，主线程经消息协议投递脚本。
//
// 文件数据面 = **OPFS 影子权威 + write-behind**：
//   - worker 启动时把 OPFS 树装载进影子索引（内容/目录/元数据）；
//   - Tcl 同步操作（open/gets/puts/file/…）即时改影子（sync 契约无
//     阻塞），同时入持久化队列；
//   - worker 让出事件循环时异步回写 OPFS（createWritable 逐操作重放）。
//   持久化时点 = exec 返回后的首个微任务；严格 durable（同步等 OPFS）
//   需 SAB+Atomics.wait 桥（v3，见 HANDOFF）。元数据（mtime/atime）
//   由影子持有——file mtime name time 等 settimes 全支持（OPFS 本身
//   无 utimes API，影子是唯一权威）。
//
// 消息协议：
//   主线程 → worker: {id, script}
//   worker → 主线程: {type:'ready'} | {id, ok, result?, error?, output}

import init, { RtclHandle } from '../pkg/rtcl_wasm.js';

let handle = null;
let outputs = [];
let outHandler = null;

// ── OPFS 影子文件系统 ────────────────────────────────────────────────

function makeOpfsFs(root) {
  const files = new Map();   // path -> {bytes: Uint8Array, mtime, atime}
  const dirs = new Set(['/']);
  let cwd = '/';
  const queue = [];          // 持久化队列：{op:'put'|'rm'|'mkdir', path, bytes?}
  let flushing = false;

  const norm = (p) => {
    if (!p.startsWith('/')) p = (cwd === '/' ? '' : cwd) + '/' + p;
    const parts = [];
    for (const seg of p.split('/')) {
      if (seg === '' || seg === '.') continue;
      if (seg === '..') parts.pop(); else parts.push(seg);
    }
    return '/' + parts.join('/');
  };
  const parent = (p) => { const i = p.lastIndexOf('/'); return i <= 0 ? '/' : p.slice(0, i); };
  const base = (p) => p.slice(p.lastIndexOf('/') + 1);
  const touchDir = (p) => { let d = norm(p); while (d && d !== '/') { dirs.add(d); d = parent(d); } };
  const now = () => Math.floor(Date.now() / 1000);
  const enc = (s) => { const u = new Uint8Array(s.length); for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i) & 0xff; return u; };
  const dec = (u) => { let s = ''; for (let i = 0; i < u.length; i++) s += String.fromCharCode(u[i]); return s; };

  function enqueue(op) { queue.push(op); scheduleFlush(); }
  function scheduleFlush() {
    if (flushing) return;
    flushing = true;
    queueMicrotask(() => { drain().finally(() => { flushing = false; if (queue.length) scheduleFlush(); }); });
  }
  async function drain() {
    while (queue.length) {
      const op = queue.shift();
      try {
        if (op.op === 'put') {
          const dh = await dirHandle(parent(op.path), true);
          const fh = await dh.getFileHandle(base(op.path), { create: true });
          const w = await fh.createWritable();
          await w.write(op.bytes);
          await w.close();
        } else if (op.op === 'rm') {
          const dh = await dirHandle(parent(op.path), false);
          await dh.removeEntry(base(op.path), { recursive: true });
        } else if (op.op === 'mkdir') {
          await dirHandle(op.path, true);
        }
      } catch (e) {
        // 回写失败不阻断影子语义（影子是权威）；留痕供诊断。
        console.warn('[opfs] flush failed for', op.op, op.path, e && e.message);
      }
    }
  }
  async function dirHandle(path, create) {
    let cur = root;
    for (const seg of norm(path).split('/').filter(Boolean)) {
      cur = await cur.getDirectoryHandle(seg, { create });
    }
    return cur;
  }
  async function loadTree() {
    async function walk(dh, prefix, depth) {
      if (depth > 8) return;
      for await (const [name, h] of dh.entries()) {
        const p = prefix + '/' + name;
        if (h.kind === 'directory') {
          dirs.add(p);
          await walk(h, p, depth + 1);
        } else {
          const f = await h.getFile();
          const bytes = new Uint8Array(await f.arrayBuffer());
          files.set(p, { bytes, mtime: Math.floor(f.lastModified / 1000), atime: Math.floor(f.lastModified / 1000) });
        }
      }
    }
    await walk(root, '', 0);
  }

  const statFn = (p) => {
    p = norm(p);
    if (files.has(p)) {
      const f = files.get(p);
      return { size: f.bytes.length, mtime: f.mtime, atime: f.atime, dir: false, file: true };
    }
    if (dirs.has(p)) return { size: 0, mtime: 0, atime: 0, dir: true, file: false };
    return null;
  };

  const contract = {
    open(path, mode) {
      path = norm(path);
      const m = mode.toLowerCase();
      const exists = files.has(path);
      if (m.startsWith('r') && !exists) {
        throw new Error("couldn't open \"" + path + "\": no such file or directory");
      }
      if (!exists) {
        files.set(path, { bytes: new Uint8Array(0), mtime: now(), atime: now() });
        enqueue({ op: 'put', path, bytes: new Uint8Array(0) });
        touchDir(parent(path));
        enqueue({ op: 'mkdir', path: parent(path) });
      }
      const entry = files.get(path);
      let pos = 0;
      if (m.startsWith('a')) pos = entry.bytes.length;
      if (m.startsWith('w')) {
        entry.bytes = new Uint8Array(0);
        entry.mtime = now();
      }
      const persist = () => {
        entry.mtime = now();
        enqueue({ op: 'put', path, bytes: entry.bytes.slice() });
      };
      return {
        read(n) {
          if (pos >= entry.bytes.length) return '';
          const take = Math.min(n || 1, entry.bytes.length - pos);
          let s = '';
          for (let i = 0; i < take; i++) s += String.fromCharCode(entry.bytes[pos + i]);
          pos += take;
          return s;
        },
        readline() {
          if (pos >= entry.bytes.length) return null;
          let s = '';
          while (pos < entry.bytes.length) {
            const c = String.fromCharCode(entry.bytes[pos++]);
            if (c === '\n') return s.replace(/\r$/, '');
            s += c;
          }
          return s;
        },
        readall() { return dec(entry.bytes); },
        write(s) {
          const add = enc(s);
          if (pos + add.length > entry.bytes.length) {
            const bigger = new Uint8Array(pos + add.length);
            bigger.set(entry.bytes, 0);
            entry.bytes = bigger;
          }
          entry.bytes.set(add, pos);
          pos += add.length;
          persist();
        },
        flush() { persist(); },
        seek(p, whence) {
          const len = entry.bytes.length;
          if (whence === 'set') pos = p;
          else if (whence === 'cur') pos += p;
          else pos = len + p;
          pos = Math.max(0, Math.min(pos, len));
          return pos;
        },
        tell() { return pos; },
        eof() { return pos >= entry.bytes.length; },
        close() { persist(); },
      };
    },
    stat(path) { return statFn(path); },
    access(path) {
      path = norm(path);
      const ok = files.has(path) || dirs.has(path);
      return { read: ok, write: true, exec: false };
    },
    mkdir(path, all) {
      path = norm(path);
      if (!all && !dirs.has(parent(path))) {
        throw new Error('couldn\'t create directory "' + path + '": no such file or directory');
      }
      touchDir(path);
      enqueue({ op: 'mkdir', path });
    },
    delete(path) {
      path = norm(path);
      if (dirs.has(path)) {
        for (const k of [...files.keys()]) if (k.startsWith(path + '/')) { files.delete(k); enqueue({ op: 'rm', path: k }); }
        for (const k of [...dirs]) if (k.startsWith(path + '/')) dirs.delete(k);
        dirs.delete(path);
        enqueue({ op: 'rm', path });
      } else if (files.has(path)) {
        files.delete(path);
        enqueue({ op: 'rm', path });
      }
    },
    rename(src, dst) {
      src = norm(src); dst = norm(dst);
      if (!files.has(src)) throw new Error('no such file');
      files.set(dst, files.get(src));
      files.delete(src);
      enqueue({ op: 'put', path: dst, bytes: files.get(dst).bytes.slice() });
      enqueue({ op: 'rm', path: src });
    },
    copy(src, dst) {
      src = norm(src); dst = norm(dst);
      if (!files.has(src)) throw new Error('no such file');
      files.set(dst, { bytes: files.get(src).bytes.slice(), mtime: now(), atime: now() });
      enqueue({ op: 'put', path: dst, bytes: files.get(dst).bytes.slice() });
    },
    readdir(path) {
      path = norm(path);
      const out = [];
      for (const k of dirs) if (parent(k) === path && k !== '/') out.push(base(k));
      for (const k of files.keys()) if (parent(k) === path) out.push(base(k));
      return out;
    },
    pwd() { return cwd; },
    cd(path) { cwd = norm(path); },
    readfile(path) {
      path = norm(path);
      if (!files.has(path)) throw new Error("couldn't read file \"" + path + "\": no such file or directory");
      return dec(files.get(path).bytes);
    },
    tempfile(prefix) {
      const p = '/tmp/' + prefix + (Date.now() % 100000);
      touchDir('/tmp');
      files.set(p, { bytes: new Uint8Array(0), mtime: now(), atime: now() });
      enqueue({ op: 'put', path: p, bytes: new Uint8Array(0) });
      return p;
    },
    settimes(path, atime, mtime) {
      path = norm(path);
      if (!files.has(path)) throw new Error("couldn't set time for \"" + path + "\": no such file");
      const f = files.get(path);
      if (atime !== null && atime !== undefined) f.atime = Number(atime);
      if (mtime !== null && mtime !== undefined) f.mtime = Number(mtime);
    },
    readlink() { throw new Error('invalid argument'); },
    statfull(path, follow) {
      const s = statFn(path);
      if (s === null) return null;
      let ino = 0;
      for (let i = 0; i < path.length; i++) ino = (ino * 31 + path.charCodeAt(i)) >>> 0;
      const mode = s.dir ? 0o40755 : 0o100644;
      return Object.assign({}, s, {
        ctime: s.mtime, dev: 0, ino, nlink: 1, uid: 1000, gid: 1000,
        mode, blksize: 4096, blocks: Math.ceil(s.size / 512), link: false,
      });
    },
  };

  return { contract, loadTree };
}

// ── worker 主体 ─────────────────────────────────────────────────────

(async () => {
  await init();
  handle = new RtclHandle();
  outHandler = (s) => outputs.push(s);
  handle.set_output_handler(outHandler);

  let fsNote = 'memory';
  try {
    if (typeof navigator !== 'undefined' && navigator.storage && navigator.storage.getDirectory) {
      const root = await navigator.storage.getDirectory();
      const fs = await makeOpfsFs(root);
      await fs.loadTree();
      handle.set_fs(fs.contract);
      fsNote = 'opfs';
    }
  } catch (e) {
    console.warn('[worker] OPFS unavailable, memory fs fallback:', e && e.message);
  }

  handle.set_output_handler((s) => outputs.push(s));
  self.postMessage({ type: 'ready', fs: fsNote });

  self.onmessage = (ev) => {
    const { id, script } = ev.data;
    outputs = [];
    try {
      const result = handle.exec(script);
      self.postMessage({ id, ok: true, result, output: outputs.join('') });
    } catch (e) {
      self.postMessage({ id, ok: false, error: String(e), output: outputs.join('') });
    }
  };
})();
