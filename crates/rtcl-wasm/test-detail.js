// ui-runtime Kv / Tree 渲染器的 node 冒烟：迷你 DOM shim。
// 校验：kv 键序/增量单行/删除/错误回路；tree 折叠预览/逐层展开/
// 深度缩进/模型更新重绘/根标量与根数组。用法：node test-detail.js。
// 浏览器行为验收另做（www/ui-detail-acc.html）。

function classList(el) { return el.className ? el.className.split(/\s+/) : []; }
function elMatches(el, sel) {
  if (!sel.startsWith('.') && !sel.startsWith('#')) return false;
  const want = sel.slice(1);
  return classList(el).includes(want) || el.dataset.uiPath === want;
}
function makeEl(tag) {
  const el = {
    tagName: tag.toUpperCase(), children: [], className: '', dataset: {}, style: {},
    textContent: '', _parent: null, clientHeight: 280, type: undefined, listeners: {},
    appendChild(c) { c._parent = el; el.children.push(c); return c; },
    append(...cs) { for (const c of cs) { c._parent = el; el.children.push(c); } },
    remove() { if (el._parent) el._parent.children = el._parent.children.filter(x => x !== el); },
    replaceChildren(...cs) { el.children = []; for (const c of cs) { c._parent = el; el.children.push(c); } },
    addEventListener(t, f) { (el.listeners[t] ??= []).push(f); },
    dispatchEv(t, ev) {
      ev.target = ev.target ?? el;
      let cur = el;
      while (cur) {
        for (const f of cur.listeners[t] || []) f(ev);
        cur = cur._parent;
      }
    },
    closest(sel) { let cur = el; while (cur) { if (elMatches(cur, sel)) return cur; cur = cur._parent; } return null; },
    classList: {
      add(c) { if (!classList(el).includes(c)) el.className = (el.className ? el.className + ' ' : '') + c; },
      remove(c) { el.className = classList(el).filter(x => x !== c).join(' '); },
      contains(c) { return classList(el).includes(c); },
    },
    head: undefined,
  };
  if (tag === 'button') el.type = 'button';
  return el;
}
globalThis.document = {
  createElement(tag) { return makeEl(tag); },
  head: makeEl('head'),
};
globalThis.requestAnimationFrame = (cb) => { cb(0); return 0; };

const { createUiRuntime, registerDomRenderers, injectUiStyles } = await import('./www/ui-runtime.js');

let fails = 0;
const ok = (cond, label, d) => {
  if (cond) console.log(`  ok  ${label}`);
  else { fails++; console.error(`FAIL  ${label}${d !== undefined ? ' :: ' + JSON.stringify(d) : ''}`); }
};

const errors = [];
const root = makeEl('div');
const runtime = createUiRuntime(root, { fire() {}, onError: (m) => errors.push(m) });
registerDomRenderers(runtime);
injectUiStyles();

// ── Kv ────────────────────────────────────────────────────────
runtime.applyOps([{
  op: 'create', path: '.kv', type: 'Kv',
  props: { bind: '/info', keys: ['name', 'status', 'uptime'] }, events: [],
}]);
const k = root.children[0];
const ks = k._kv;
ok(ks && ks.rows.size === 0, 'kv: empty until data');

const rowTexts = () => [...ks.rows.entries()].map(([key, r]) => `${key}=${r.vEl.textContent}`);
runtime.applyOps([{
  op: 'model', sub: 'set', path: '/info',
  value: { name: 'pod-42', status: 'Running', uptime: 123, nested: { a: 1 }, ok: true, none: null },
}]);
ok(rowTexts().join('|') === 'name=pod-42|status=Running|uptime=123', 'kv: -keys order, unlisted keys skipped', rowTexts());
ok(ks.rows.size === 3, 'kv: row cache bounded to listed keys');

// 单键增量：节点身份保持
const statusRow = ks.rows.get('status').row;
const nameRow = ks.rows.get('name').row;
runtime.applyOps([{ op: 'model', sub: 'set', path: '/info/status', value: 'CrashLoopBackOff' }]);
ok(ks.rows.get('status').row === statusRow, 'kv: single-key update reuses row node');
ok(ks.rows.get('status').vEl.textContent === 'CrashLoopBackOff', 'kv: updated value in place');
ok(ks.rows.get('name').row === nameRow, 'kv: other rows untouched');

// unset 单键 → 整行移除
runtime.applyOps([{ op: 'model', sub: 'unset', path: '/info/uptime' }]);
ok(ks.rows.size === 2 && !ks.rows.has('uptime'), 'kv: unset removes the row');

// 默认键序 + 标量格式化
runtime.applyOps([{ op: 'create', path: '.kv2', type: 'Kv', props: { bind: '/m2' }, events: [] }]);
const k2 = root.children[1]._kv;
runtime.applyOps([{ op: 'model', sub: 'set', path: '/m2', value: { z: 1, a: [1, 2], s: 'x', b: true, n: null } }]);
const t2 = [...k2.rows.entries()].map(([key, r]) => `${key}=${r.vEl.textContent}`).join('|');
ok(t2 === 'z=1|a=[1,2]|s=x|b=true|n=null', 'kv: default insertion order + compact JSON/bool/null', t2);

// 非对象 → onError（不静默）
runtime.applyOps([{ op: 'model', sub: 'set', path: '/info', value: 'not-an-object' }]);
ok(errors.length === 1 && errors[0].includes('must be a JSON object'), 'kv: non-object -> onError', errors);

// ── Tree ──────────────────────────────────────────────────────
runtime.applyOps([{ op: 'create', path: '.tr', type: 'Tree', props: { bind: '/obj' }, events: [] }]);
const tr = root.children[2];
const ts = tr._tree;
ok(ts && tr.children.length === 0, 'tree: empty until data');

const rows = () => ts.el.children;
const rowAt = (i) => rows()[i];
const clickRow = (i) => rowAt(i).dispatchEv('click', { target: rowAt(i) });
const labels = () => rows().map(r => (r.children[1]?.textContent ?? '') + (r.children[2] ? ' ' + r.children[2].textContent : ''));
runtime.applyOps([{
  op: 'model', sub: 'set', path: '/obj',
  value: { a: 1, b: { c: 2, d: [3, 4] }, e: [{ f: 5 }] },
}]);
ok(rows().length === 1 && labels()[0] === '(root) {…} 3 keys', 'tree: root collapsed with preview', labels());

clickRow(0);   // 展开 root
ok(rows().length === 4, 'tree: root expand → one line per child', rows().length);
ok(labels().join('|') === '(root)|a 1|b {…} 2 keys|e […] 1 items', 'tree: child labels + collapsed previews', labels());
ok(rowAt(1).style.paddingLeft === '14px', 'tree: depth indent applied', rowAt(1).style.paddingLeft);

const idxB = rows().map(r => r.children[1]?.textContent).indexOf('b');
clickRow(idxB);   // 展开 b
ok(rows().length === 6, 'tree: nested object expand', rows().length);
const dIdx = rows().map(r => r.children[1]?.textContent).indexOf('d');
ok(rowAt(dIdx).children[2]?.textContent.includes('2 items'), 'tree: array preview', rowAt(dIdx).children[2]?.textContent);
clickRow(dIdx);   // 展开 d
ok(rows().length === 8 && labels().some(l => l === '[0] 3') && labels().some(l => l === '[1] 4'), 'tree: array items [i]', labels());

// 收起 b → 回到 4 行（懒物化，DOM 缩回）
clickRow(rows().map(r => r.children[1]?.textContent).indexOf('b'));
ok(rows().length === 4, 'tree: collapse releases descendant DOM', rows().length);

// 模型子树更新：收起态下重绘但保持行数；展开后读到新值
runtime.applyOps([{ op: 'model', sub: 'set', path: '/obj/b/c', value: 9 }]);
ok(rows().length === 4, 'tree: update while collapsed keeps visible lines', rows().length);
clickRow(rows().map(r => r.children[1]?.textContent).indexOf('b'));
ok(labels().some(l => l === 'c 9'), 'tree: update visible after expand', labels());

// 根标量 + 根数组
runtime.applyOps([{ op: 'create', path: '.tr2', type: 'Tree', props: { bind: '/scalar' }, events: [] }]);
const ts2 = root.children[3]._tree;
runtime.applyOps([{ op: 'model', sub: 'set', path: '/scalar', value: 42 }]);
ok(ts2.el.children.length === 1 && ts2.el.children[0].children[2]?.textContent === '42', 'tree: scalar root', ts2.el.children[0]?.children.map(c => c.textContent));
runtime.applyOps([{ op: 'create', path: '.tr3', type: 'Tree', props: { bind: '/arr' }, events: [] }]);
const ts3 = root.children[4]._tree;
runtime.applyOps([{ op: 'model', sub: 'set', path: '/arr', value: [7, 8] }]);
ok(ts3.el.children.length === 1 && ts3.el.children[0].children[2]?.textContent.includes('2 items'), 'tree: array root preview');
ts3.el.children[0].dispatchEv('click', { target: ts3.el.children[0] });
ok(ts3.el.children.length === 3, 'tree: array root expand', ts3.el.children.length);

// 整树 unset → 清空
runtime.applyOps([{ op: 'model', sub: 'unset', path: '/obj' }]);
ok(rows().length === 0, 'tree: unset clears the tree');

const done = fails === 0;
console.log(done ? 'SMOKE OK' : `SMOKE FAILED (${fails})`);
process.exit(done ? 0 : 1);
