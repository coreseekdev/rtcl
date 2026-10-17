// ui-runtime Table renderer 的 node 冒烟：迷你 DOM shim + 同步 rAF。
// 校验：虚拟滚动窗口尺寸、行级增量不重建其他节点、事件委托、错误回路。
// 用法：node test-table.js   （纯 JS，不需要 wasm 构建）
// 浏览器行为验收另做（www/ui.html demo）。

// ── 迷你 DOM ──────────────────────────────────────────────────
function classList(el) { return el.className ? el.className.split(/\s+/) : []; }
function elMatches(el, sel) {
  if (!sel.startsWith('.') && !sel.startsWith('#')) return false;
  const want = sel.slice(1);
  return classList(el).includes(want) || el.dataset.uiPath === want;
}
function makeEl(tag) {
  const el = {
    tagName: tag.toUpperCase(), children: [], className: '', dataset: {}, style: {},
    textContent: '', _parent: null, scrollTop: 0, clientHeight: 280,
    type: undefined, listeners: {},
    appendChild(c) { c._parent = el; el.children.push(c); return c; },
    append(...cs) { for (const c of cs) { c._parent = el; el.children.push(c); } },
    remove() { if (el._parent) el._parent.children = el._parent.children.filter(x => x !== el); },
    replaceChildren(...cs) { el.children = []; for (const c of cs) { c._parent = el; el.children.push(c); } },
    addEventListener(t, f) { (el.listeners[t] ??= []).push(f); },
    dispatchEv(t, ev) {
      ev.target = ev.target ?? el;
      let cur = el;   // 冒泡：从 target 沿 _parent 链走（JS DOM 语义）
      while (cur) {
        for (const f of cur.listeners[t] || []) f(ev);
        cur = cur._parent;
      }
    },
    closest(sel) { let cur = el; while (cur) { if (elMatches(cur, sel)) return cur; cur = cur._parent; } return null; },
    head: undefined,
  };
  if (tag === 'button') el.type = 'button';
  return el;
}
const rafQ = [];
globalThis.document = {
  createElement(tag) { return makeEl(tag); },
  head: makeEl('head'),
};
globalThis.requestAnimationFrame = (cb) => { rafQ.push(cb); return rafQ.length; };
function flushRaf() { while (rafQ.length) { const cb = rafQ.shift(); cb(); } }

const { createUiRuntime, registerDomRenderers, injectUiStyles } = await import('./www/ui-runtime.js');

let fails = 0;
const ok = (cond, label, d) => {
  if (cond) console.log(`  ok  ${label}`);
  else { fails++; console.error(`FAIL  ${label}${d !== undefined ? ' :: ' + JSON.stringify(d) : ''}`); }
};

const fired = [];
const errors = [];
const root = makeEl('div');
const runtime = createUiRuntime(root, {
  fire: (path, event, detail) => fired.push([path, event, detail]),
  onError: (m) => errors.push(m),
});
registerDomRenderers(runtime);

// 建表
runtime.applyOps([{
  op: 'create', path: '.t', type: 'Table',
  props: { bind: '/rows', columns: ['name', 'status'] },
  events: ['rowopen', 'rowclick'],
}]);
const t = root.children[0];
const st = t._table;
ok(st && st.cols.length === 2, 'create: columns parsed');
ok(st.n === 0, 'create: empty until data');
injectUiStyles();
ok(document.head.children.length === 1, 'styles injectable');

// 喂 200 行（游标：一次一行）
for (let i = 0; i < 200; i++) {
  runtime.applyOps([{ op: 'model', sub: 'set', path: `/rows/${i}`, value: { name: `pod-${i}`, status: i % 2 ? 'Running' : 'Pending' } }]);
}
ok(st.rows.length === 200 && st.rows[42].name === 'pod-42', 'cursor feed fills row cache');
ok(st.pool.size <= 26 + 1, 'virtual window bounded (viewport+buffer)', st.pool.size);
ok(!st.pool.has(42), 'row 42 outside window not rendered');
console.log(`  ..  pool.size=${st.pool.size} n=${st.n}`);

// 滚到 42 附近 → 窗口行
t.scrollTop = 42 * 28;
t.dispatchEv('scroll', {});
flushRaf();
ok(st.pool.has(42), 'scroll brings row 42 into window');

// 单行增量：节点身份保持
const before42 = st.pool.get(42);
const otherList = [...st.pool.keys()];
runtime.applyOps([{ op: 'model', sub: 'set', path: '/rows/42', value: { name: 'pod-42', status: 'CrashLoopBackOff' } }]);
const after42 = st.pool.get(42);
ok(before42 === after42, 'single-row update reuses row node');
const cellTxt = [...before42._cells.map(c => c.textContent)].join('|');
ok(cellTxt === 'pod-42|CrashLoopBackOff', 'updated row text in place', cellTxt);
const sameKeys = [...st.pool.keys()].join(',') === otherList.join(',');
ok(sameKeys, 'visibility unchanged for other rows');

// 全量 set（初始数组形态）也走通
runtime.applyOps([{ op: 'model', sub: 'set', path: '/other', value: [{ name: 'x' }] }]);
// 不动 .t（未订阅 /other）

// unset 行：置洞
runtime.applyOps([{ op: 'model', sub: 'unset', path: '/rows/41' }]);
ok(st.rows[41] === undefined, 'unset leaves hole in row cache');
const r41 = st.pool.get(41);
ok(r41 === undefined || r41._cells[0].textContent === '', 'hole renders empty row');

// 事件委托：行内 act 按钮 + 行身（target = 按钮/单元格，冒泡到表容器）
const rowEl = st.pool.get(42);
const zone = rowEl.children[rowEl.children.length - 1];
const btn = zone.children[0];
rowEl.dispatchEv('click', { target: btn });
ok(fired.some(([p, ev, idx]) => p === '.t' && ev === 'rowopen' && String(idx) === '42'), 'act button fires rowopen with detail', fired[fired.length - 1]);
fired.length = 0;
rowEl.dispatchEv('click', { target: rowEl });
ok(fired[0]?.[1] === 'rowclick' && String(fired[0]?.[2]) === '42', 'row body fires rowclick', fired);

// 渲染器错误走 onError（非数组喂入）
runtime.applyOps([{
  op: 'create', path: '.bad', type: 'Table', props: { bind: '/x', columns: ['k'] },
}]);
runtime.applyOps([{ op: 'model', sub: 'set', path: '/x', value: 'not-an-array' }]);
ok(errors.length === 1 && errors[0].includes('must be a JSON array'), 'non-array rows -> onError', errors);

// ui::on 追加动作（events 变化 → 行节点重建带按钮）
fired.length = 0;
runtime.applyOps([{ op: 'set', path: '.t', props: {}, events: ['rowopen', 'rowdel', 'rowclick'] }]);
const row42 = st.pool.get(42);
ok(st.actions.join(',') === 'open,del', 'ui::on adds action set', st.actions);
const nb = row42.children.find(c => classList(c).includes('rui-td-acts'));
ok(nb && nb.children.length === 2, 're-rendered rows carry action buttons');
nb.dispatchEv('click', { target: nb.children[1] });   // del 按钮（真 .rui-act）
ok(fired[0]?.[0] === '.t' && fired[0]?.[1] === 'rowdel', 'second action fires rowdel', fired);

const done = fails === 0;
console.log(done ? 'SMOKE OK' : `SMOKE FAILED (${fails})`);
process.exit(done ? 0 : 1);
