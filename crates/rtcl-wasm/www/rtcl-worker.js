// 浏览器 worker 原生 postMessage/onmessage；node worker_threads 测试链
// 路由 parentPort 提供（self/全局兼容垫片）。
const BOOT_QUEUE = [];
let booted = false;
let __nodePort = null;
if (typeof postMessage !== 'function') {
  // node worker_threads（本仓测试链路）——ESM 顶层 await
  const { parentPort } = await import('node:worker_threads');
  globalThis.postMessage = (m) => parentPort.postMessage(m);
  parentPort.on('message', (m) => globalThis.__recv({ data: m }));
}
// rtcl-worker.js — rtcl 的 worker 部署形态（网页 dispatcher 的真实运行
// 结构）：解释器 + JS 桥全部在本 worker，主线程经消息协议投递脚本。
//
// 版本参数（页面 goto 带 ?v=）穿透 HTTP/模块缓存层——pkg 内容变更
// 时同 URL 旧缓存可能钉住 import。动态 import（模块 worker 支持 TLA）。
const params = new URLSearchParams(self.location?.search || '');
const PKG_V = params.get('v') || '';
const { default: init, RtclHandle } = await import('../pkg/rtcl_wasm.js' + (PKG_V ? '?v=' + PKG_V : ''));
import { makeRegexpMod } from './regexpmod.js';
import { makeEncodingMod } from './encodingmod.js';

// regexp 外置的语义 wrapper proc（::regexp/::regsub 解包结构化编码并
// upvar 写变量；native 同面）。
const REGEXP_SETUP_TCL = `
proc ::regexp {args} {
  set r [::regexp::regexp [join $args "\u0001"]]
  if {[lindex $r 0] eq "__RXCAP"} {
    set k [lindex $r 1]
    set vals [lrange $r 2 end]
    set i 0
    foreach name [lrange $args [expr {[llength $args] - $k}] end] {
      upvar 1 $name v
      set v [lindex $vals $i]
      incr i
    }
    return 1
  }
  if {[lindex $r 0] eq "__RXIDX"} {
    set k [lindex $r 1]
    set pairs [lrange $r 2 end]
    set i 0
    foreach name [lrange $args [expr {[llength $args] - $k}] end] {
      upvar 1 $name v
      set v [lindex $pairs $i]
      incr i
    }
    return 1
  }
  return $r
}
proc ::regsub {args} {
  # JS 面的 regsub fn 装在 ::regexp 模块下（::regexp::regsub）
  set r [::regexp::regsub [join $args "\u0001"]]
  if {[lindex $r 0] eq "__RS"} {
    upvar 1 [lindex $args end] v
    set v [lindex $r 2]
    return [lindex $r 1]
  }
  return $r
}
`;

// ── spencermod：Spencer 全集侧模块（native 同源引擎）─────────────────
// ::spencermod::exec exp string flags start all →
//   平面列表 "nmatches ngroups {b e} {b e} ..."（字节跨度，端点独占）；
//   编译错误由 JS 胶水 throw（native 同文前缀）。
// wrapper 职责：选项解析、变量写入（-all 时取**末匹配**——native
// set_match_var_groups 同）、regsub 替换。span 为字符单位（side 模块
// 直出；-indices 与 string range 同单位，native byte_to_char 后的
// 用户面同约定），虚拟起点 -start 的索引 delta 由 wrapper 补。
const SPENCER_SETUP_TCL = `
proc ::_spencer_match_vals {string spans m ng indices delta} {
  set vals {}
  set base [expr {2 * $m * $ng}]
  for {set g 0} {$g < $ng} {incr g} {
    set cs [lindex $spans [expr {$base + 2 * $g}]]
    set ce [lindex $spans [expr {$base + 2 * $g + 1}]]
    if {$cs < 0} {
      if {$indices} { lappend vals [list [expr {-1 + $delta}] [expr {-1 + $delta}]] } else { lappend vals "" }
      continue
    }
    if {$indices} {
      lappend vals [list [expr {$cs + $delta}] [expr {$ce - 1 + $delta}]]
    } else {
      if {$ce > $cs} {
        lappend vals [string range $string $cs [expr {$ce - 1}]]
      } else {
        lappend vals ""
      }
    }
  }
  return $vals
}
# 仅可从 ::regexp 调用：upvar 2 = 越过本 helper 与 ::regexp，直达
# 用户的调用帧（upvar 1 会落在 ::regexp 局部帧上，变量写丢）。
proc ::_spencer_write_vars {capVars vals ng} {
  if {[llength $capVars] == 0} { return }
  upvar 2 [lindex $capVars 0] mv
  set mv [lindex $vals 0]
  set gi 1
  foreach name [lrange $capVars 1 end] {
    upvar 2 $name sv
    if {$gi < $ng} { set sv [lindex $vals $gi] } else { set sv "" }
    incr gi
  }
}
proc ::_spencer_parse_opts {argsVar bools withStart extra} {
  upvar 1 $argsVar args
  array set o [list]
  foreach b $bools { set o($b) 0 }
  set o(start) 0
  set i 0
  set n [llength $args]
  while {$i < $n} {
    set a [lindex $args $i]
    if {[lsearch -exact $bools [string range $a 1 end]] >= 0 && [string index $a 0] eq "-"} {
      set o([string range $a 1 end]) 1
      if {$a eq "-line"} { set o(lineanchor) 1; set o(linestop) 1 }
    } elseif {$withStart && $a eq "-start"} {
      incr i
      if {$i >= $n} { error "missing argument to \\"-start\\"" }
      set o(start) [lindex $args $i]
    } elseif {$a eq "--"} {
      incr i
      break
    } elseif {[string index $a 0] eq "-"} {
      error "bad option \\"$a\\": must be -about, -all, -expanded, -indices, -inline, -line, -linestop, -lineanchor, -nocase, -start, or --"
    } else {
      break
    }
    incr i
  }
  set o(rest) [lrange $args $i end]
  return [array get o]
}
proc ::regexp {args} {
  set p [::_spencer_parse_opts args [list nocase line lineanchor linestop expanded all inline indices about] 1 0]
  array set o $p
  set rest $o(rest)
  if {[llength $rest] < 2 && !$o(about)} {
    error {wrong # args: should be "regexp ?switches? exp string ?matchVar? ?subMatchVar ...?"}
  }
  set exp [lindex $rest 0]
  set string [lindex $rest 1]
  set capVars [lrange $rest 2 end]
  if {$o(about)} {
    set r [list $exp]
    if {[string index $exp 0] eq "^"} { lappend r anchor-start }
    set dollar [format %c 36]
    if {[string index $exp end] eq $dollar} { lappend r anchor-end }
    return $r
  }
  set flags [expr {$o(nocase) + 2 * $o(linestop) + 4 * $o(lineanchor) + 8 * $o(expanded)}]
  set r [::spencermod::exec $exp $string $flags $o(start) $o(all)]
  set nm [lindex $r 0]
  set ng [lindex $r 1]
  set spans [lrange $r 2 end]
  if {$nm == 0} { return 0 }
  # 虚拟起点（-start 越过串尾）：索引按原始 -start 计（native
  # index_delta 同）。
  set charLen [string length $string]
  set eff [expr {$o(start) < $charLen ? $o(start) : $charLen}]
  set delta [expr {$o(start) - $eff}]
  if {$o(all) && $o(inline)} {
    set out {}
    for {set m 0} {$m < $nm} {incr m} {
      foreach v [::_spencer_match_vals $string $spans $m $ng 0 $delta] { lappend out $v }
    }
    return $out
  }
  if {$o(all) && $o(indices)} {
    set out {}
    for {set m 0} {$m < $nm} {incr m} {
      foreach v [::_spencer_match_vals $string $spans $m $ng 1 $delta] { lappend out $v }
    }
    return $out
  }
  if {$o(all)} {
    # 变量取**末匹配**（native set_match_var_groups 同），返回计数
    set vals [::_spencer_match_vals $string $spans [expr {$nm - 1}] $ng 0 $delta]
    ::_spencer_write_vars $capVars $vals $ng
    return $nm
  }
  if {$o(inline)} { return [::_spencer_match_vals $string $spans 0 $ng 0 $delta] }
  if {$o(indices)} {
    set pairs [::_spencer_match_vals $string $spans 0 $ng 1 $delta]
    if {[llength $capVars] > 0} {
      ::_spencer_write_vars $capVars $pairs $ng
      return 1
    }
    return $pairs
  }
  set vals [::_spencer_match_vals $string $spans 0 $ng 0 $delta]
  ::_spencer_write_vars $capVars $vals $ng
  return 1
}
proc ::_spencer_expand {spec vals} {
  set out ""
  set i 0
  set L [string length $spec]
  set bs [format %c 92]
  while {$i < $L} {
    set c [string index $spec $i]
    if {$c eq "&"} {
      append out [lindex $vals 0]
      incr i
    } elseif {$c eq $bs && $i + 1 < $L} {
      set nx [string index $spec [expr {$i + 1}]]
      if {$nx >= "0" && $nx <= "9"} {
        append out [lindex $vals $nx]
        incr i 2
      } elseif {$nx eq $bs} {
        append out $bs
        incr i 2
      } else {
        append out $c
        incr i
      }
    } else {
      append out $c
      incr i
    }
  }
  return $out
}
proc ::regsub {args} {
  set p [::_spencer_parse_opts args [list nocase all expanded line lineanchor linestop] 1 0]
  array set o $p
  set rest $o(rest)
  if {[llength $rest] < 3} {
    error {wrong # args: should be "regsub ?switches? exp string subSpec varName"}
  }
  set exp [lindex $rest 0]
  set string [lindex $rest 1]
  set subSpec [lindex $rest 2]
  set varName [lindex $rest 3]
  set flags [expr {$o(nocase) + 2 * $o(linestop) + 4 * $o(lineanchor) + 8 * $o(expanded) + 16}]
  set r [::spencermod::exec $exp $string $flags $o(start) 1]
  set nm [lindex $r 0]
  set ng [lindex $r 1]
  set spans [lrange $r 2 end]
  set out ""
  set pos 0
  for {set m 0} {$m < $nm} {incr m} {
    set cb [lindex $spans [expr {2 * $m * $ng}]]
    set ce [lindex $spans [expr {2 * $m * $ng + 1}]]
    if {$cb < 0} { continue }
    if {$cb > $pos} { append out [string range $string $pos [expr {$cb - 1}]] }
    set vals [::_spencer_match_vals $string $spans $m $ng 0 0]
    append out [::_spencer_expand $subSpec $vals]
    set pos $ce
  }
  if {$pos < [string length $string]} { append out [string range $string $pos end] }
  if {$varName eq ""} { return $nm }
  upvar 1 $varName v
  set v $out
  return $nm
}
`;

// spencermod 装载：fetch + instantiate wasm，注册 v2 dynmod（exec 经
// envelope 编解码调用侧模块数值导出）。失败 → 返回 false（回退
// regexpmod.js 语义子集）。
async function setupSpencerMod(handle) {
  let sp;
  const wasmUrl = 'spencermod.wasm' + (PKG_V ? '?v=' + PKG_V : '');
  try {
    const res = await WebAssembly.instantiateStreaming(fetch(wasmUrl));
    sp = res.instance.exports;
  } catch (e) {
    const buf = await (await fetch(wasmUrl)).arrayBuffer();
    sp = (await WebAssembly.instantiate(buf)).instance.exports;
  }
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const glue = {
    exec(...rest) {
      // v2 dispatch（__dynmodv2:）逐词透传——rest 即原词；保留数组/
      // v1 join 形态兼容，regexpmod normalizeArgs 同约定。
      const words = Array.isArray(rest[0])
        ? rest[0]
        : (rest.length === 1 && typeof rest[0] === 'string' && rest[0].includes('\u0001'))
          ? rest[0].split('\u0001')
          : rest;
      const parts = words.map((w) => enc.encode(String(w)));
      let total = 4;
      for (const p of parts) total += 4 + p.length;
      const inBuf = new Uint8Array(total);
      const dv = new DataView(inBuf.buffer);
      dv.setUint32(0, parts.length, true);
      let o = 4;
      for (const p of parts) {
        dv.setUint32(o, p.length, true);
        inBuf.set(p, o + 4);
        o += 4 + p.length;
      }
      const ip = sp.alloc(inBuf.length);
      new Uint8Array(sp.memory.buffer, ip, inBuf.length).set(inBuf);
      sp.exec(ip, inBuf.length);
      const rp = sp.res_ptr();
      const rl = sp.res_len();
      const out = new Uint8Array(sp.memory.buffer, rp, rl);
      const odv = new DataView(sp.memory.buffer, rp, rl);
      const tag = odv.getUint32(0, true);
      if (tag !== 0) {
        let q = 4;
        const cl = odv.getUint32(q, true); q += 4;
        const code = dec.decode(new Uint8Array(sp.memory.buffer, rp + q, cl)); q += cl;
        const ml = odv.getUint32(q, true); q += 4;
        const msg = dec.decode(new Uint8Array(sp.memory.buffer, rp + q, ml));
        void code;
        throw new Error("couldn't compile regular expression pattern: " + msg);
      }
      const ngroups = odv.getUint32(4, true);
      const nmatches = odv.getUint32(8, true);
      // 平铺数字（每对两元素）——wrapper match_vals 按
      // 2*(m*ng+g) 索引；负数 = 未参与组。
      let s = nmatches + ' ' + ngroups;
      for (let i = 0; i < nmatches * ngroups; i++) {
        const b = odv.getInt32(12 + 8 * i, true);
        const e2 = odv.getInt32(16 + 8 * i, true);
        s += ' ' + b + ' ' + e2;
      }
      return s;
    },
  };
  handle.register_module('spencermod', glue);
  try {
    handle.exec('load spencermod');
    handle.exec(SPENCER_SETUP_TCL);
  } catch (e) {
    // wrapper 安装失败（Tcl 语法/引擎缺失）→ 让位给 JS 子集回退
    return false;
  }
  return !!handle.exec('namespace which -command ::regexp')
      && !!handle.exec('namespace which -command ::regsub');
}

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

// ── SAB 同步桥（bridge 模式的 fs 契约）：同步阻塞等待 fs-worker 的
// OPFS 回执——每个 Tcl 文件操作在返回前完成 OPFS 落盘（严格 durable）。
// ── SAB 严格 durable 形态的契约构造（fs-worker 为 OPFS 执行端）────
function makeBridgeFs(sab, fsPort) {
  const i32 = new Int32Array(sab);
  const u8 = new Uint8Array(sab);
  let seq = 0;
  function callFs(op, params) {
    seq += 1;
    // BigInt（i64 实参）不可 JSON.stringify——统一 Number 归一
    const safe = JSON.stringify({ op, params }, (_k, v) =>
      typeof v === 'bigint' ? Number(v) : v);
    const req = new TextEncoder().encode(safe);
    u8.set(req.subarray(0, 256 * 1024 - 64), 64);
    Atomics.store(i32, 2, req.length);          // PAYLOAD_LEN
    Atomics.store(i32, 0, seq);                 // REQ_SEQ
    Atomics.store(i32, 1, 0);                   // REPLY_SEQ 复位
    fsPort.postMessage({ seq });
    const r = Atomics.wait(i32, 1, 0);          // 阻塞至 fs worker 回执
    if (r !== 'ok') throw new Error('fs bridge wait failed: ' + r);
    const len = Atomics.load(i32, 2);
    // SAB 视图不可直接喂 TextDecoder——拷贝
    const copy = new Uint8Array(len);
    copy.set(u8.subarray(64, 64 + len));
    const reply = JSON.parse(new TextDecoder().decode(copy));
    if (!reply.ok) throw new Error(reply.error);
    return reply.result;
  }
  // 契约面（方法名 = fs_bridge.rs 的 JsFs 契约；open 返回句柄对象，
  // JsChannel 经 FILE_OBJECTS 间接调用其 read/write/... 方法）。
  return {
    open(path, mode) {
      const fid = String(callFs('open', { path, mode }));
      return {
        read(n) { return callFs('read', { fid, n: Number(n) }); },
        readline() { return callFs('readline', { fid }); },
        readall() { return callFs('readall', { fid }); },
        write(s) { return callFs('write', { fid, data: s }); },
        flush() { return callFs('flush', { fid }); },
        seek(pos, whence) { return callFs('seek', { fid, pos: Number(pos), whence }); },
        tell() { return callFs('tell', { fid }); },
        eof() { return callFs('eof', { fid }); },
        close() { return callFs('close', { fid }); },
        _fid: fid,
      };
    },
    stat(path) { return callFs('stat', { path }); },
    statfull(path, follow) { return callFs('statfull', { path, follow }); },
    access(path) { return callFs('access', { path }); },
    mkdir(path, all) { return callFs('mkdir', { path, all }); },
    delete(path) { return callFs('delete', { path }); },
    rename(src, dst) { return callFs('rename', { src, dst }); },
    copy(src, dst) { return callFs('copy', { src, dst }); },
    readdir(path) { return callFs('readdir', { path }); },
    pwd() { return callFs('pwd', {}); },
    cd(path) { return callFs('cd', { path }); },
    readfile(path) { return callFs('readfile', { path }); },
    tempfile(prefix) { return callFs('tempfile', { prefix }); },
    settimes(path, atime, mtime) { return callFs('settimes', { path, atime, mtime }); },
    readlink(path) { return callFs('readlink', { path }); },
  };
}

// ── 模块级消息路由 ────────────────────────────────────────────────
// module worker 规范：eval 完成前父线程投递的消息由浏览器排队，eval
// 结束后才派发——所以 onmessage 必须在 eval 期间就位，消息入
// BOOT_QUEUE，IIFE 在 fs 分支判断**前**重放（否则 bridge SAB/port 到
// 达时 fs 模式已定为直连——stFs 静默降级的根因）。
let handle = null;
let bridgeSab = null;
let bridgePort = null;
let outputs = [];

function handleControl(msg) {
  if (msg.sab && msg.fsPort) {
    bridgeSab = msg.sab;
    bridgePort = msg.fsPort;
    return true;
  }
  return false;
}

function dispatchExec({ id, script }) {
  outputs = [];
  try {
    const result = handle.exec(script);
    globalThis.postMessage({ id, ok: true, result, output: outputs.join('') });
  } catch (e) {
    globalThis.postMessage({ id, ok: false, error: String(e), output: outputs.join('') });
  }
}

function dispatchAndControl(ev) {
  const msg = ev.data;
  if (msg.id !== undefined && msg.script !== undefined) dispatchExec(msg);
  else handleControl(msg);
}

globalThis.onmessage = (ev) => {
  if (booted) { dispatchAndControl(ev); return; }
  BOOT_QUEUE.push(ev.data);
};
if (typeof globalThis.__recv !== 'function') {
  globalThis.__recv = (ev) => {
    if (booted) { dispatchAndControl(ev); return; }
    BOOT_QUEUE.push(ev.data);
  };
}

// ── 启动序列 ────────────────────────────────────────────────────────

(async () => {
  try {
    await init();
    handle = new RtclHandle();
    handle.set_output_handler((s) => outputs.push(s));

    // ① 重放排队的消息：控制消息（bridge SAB/port）先于 exec
    const execQ = [];
    while (BOOT_QUEUE.length) {
      const msg = BOOT_QUEUE.shift();
      if (msg.id !== undefined && msg.script !== undefined) execQ.push(msg);
      else handleControl(msg);
    }

    // ② fs 模式判定（bridge → 直连 OPFS → 内存回落）
    let fsNote = 'memory';
    if (bridgeSab !== null) {
      // SAB 严格 durable 形态：Atomics.wait 阻塞等待 fs-worker 的
      // OPFS 回执（回执在写入完成之后发出）。契约对象经
      // RtclHandle::set_fs → JsFs 包装（方法名 = fs_bridge.rs 契约）。
      const i32 = new Int32Array(bridgeSab);
      const u8 = new Uint8Array(bridgeSab);
      let seq = 0;
      function callFs(op, params) {
        seq += 1;
        const safe = JSON.stringify({ op, params }, (_k, v) =>
          typeof v === 'bigint' ? Number(v) : v);
        const req = new TextEncoder().encode(safe);
        u8.set(req.subarray(0, 256 * 1024 - 64), 64);
        Atomics.store(i32, 2, req.length);
        Atomics.store(i32, 0, seq);
        Atomics.store(i32, 1, 0);
        bridgePort.postMessage({ seq });
        // 等待 REPLY_SEQ === seq（旧回执迟到时 not-equal 早返——循环比对
        // seq 精确匹配，读到旧回执会导致文件操作错乱）
        for (;;) {
          const cur = Atomics.load(i32, 1);
          if (cur === seq) break;
          const r = Atomics.wait(i32, 1, cur);
          if (r === 'not-equal') {
            if (Atomics.load(i32, 1) === seq) break;
          }
        }
        const len = Atomics.load(i32, 2);
        // SAB 视图不可直接喂 TextDecoder——拷贝
        const copy = new Uint8Array(len);
        copy.set(u8.subarray(64, 64 + len));
        const reply = JSON.parse(new TextDecoder().decode(copy));
        if (!reply.ok) throw new Error(reply.error);
        return reply.result;
      }
      const contract = {
        open(path, mode) {
          const h = callFs('open', { path, mode });
          const fid = String(h);
          return {
            read(n) { return callFs('read', { fid, n: Number(n) }); },
            readline() { return callFs('readline', { fid }); },
            readall() { return callFs('readall', { fid }); },
            write(s) { return callFs('write', { fid, data: s }); },
            flush() { return callFs('flush', { fid }); },
            seek(pos, whence) { return callFs('seek', { fid, pos: Number(pos), whence }); },
            tell() { return callFs('tell', { fid }); },
            eof() { return callFs('eof', { fid }); },
            close() { return callFs('close', { fid }); },
            _fid: fid,
          };
        },
        stat(path) { return callFs('stat', { path }); },
        statfull(path, follow) { return callFs('statfull', { path, follow }); },
        access(path) { return callFs('access', { path }); },
        mkdir(path, all) { return callFs('mkdir', { path, all }); },
        delete(path) { return callFs('delete', { path }); },
        rename(src, dst) { return callFs('rename', { src, dst }); },
        copy(src, dst) { return callFs('copy', { src, dst }); },
        readdir(path) { return callFs('readdir', { path }); },
        pwd() { return callFs('pwd', {}); },
        cd(path) { return callFs('cd', { path }); },
        readfile(path) { return callFs('readfile', { path }); },
        tempfile(prefix) { return callFs('tempfile', { prefix }); },
        settimes(path, atime, mtime) { return callFs('settimes', { path, atime, mtime }); },
        readlink(path) { return callFs('readlink', { path }); },
      };
      handle.set_fs(contract);
      fsNote = 'opfs-sab';
    } else {
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
    }

    handle.set_output_handler((s) => outputs.push(s));

    // regexp：优先 Spencer 全集侧模块（native 同源引擎，选项/类别/
    // 回引/lookahead 全集）；不可用（fetch/instantiate 失败）回退
    // regexpmod.js 语义子集。两者都装 ::regexp/::regsub wrapper。
    if (!handle.exec('namespace which -command ::regexp')) {
      let spencerOk = false;
      try {
        spencerOk = await setupSpencerMod(handle);
      } catch (e) {
        console.warn('[worker] spencermod unavailable, JS subset fallback:', e && e.message);
      }
      if (!spencerOk) {
        const mod = makeRegexpMod();
        handle.register_module('regexp', mod);
        handle.exec('load regexp');
        handle.exec(REGEXP_SETUP_TCL);
      }
    }
    // encoding 外置：wasm 子集不含 native encoding 命令（feature 门控）
    // ——JS TextDecoder/TextEncoder 数据面（86 名宽表，映射见
    // encodingmod.js；symbol/iso2022 系/ebcdic 如实报 unsupported）。
    if (!handle.exec('namespace which -command ::encoding')) {
      handle.register_module('encoding', makeEncodingMod());
      handle.exec('proc ::encoding {args} { ::encoding::encoding {*}[join $args "\u0001"] }');
      handle.exec('load encoding');
    }

    globalThis.postMessage({ type: 'ready', fs: fsNote });

    // ③ exec 队列补跑（bridge/direct 均就绪后）
    booted = true;
    for (const msg of execQ) dispatchExec(msg);
  } catch (e) {
    globalThis.postMessage({ type: 'init-error', error: String(e && e.stack || e) });
  }
})();
