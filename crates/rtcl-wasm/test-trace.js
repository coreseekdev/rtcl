// ui-runtime Trace 渲染器的 node 冒烟：迷你 DOM shim + 同步 rAF。
// 校验：行/缩进、条几何（全域 %）、折叠不改对齐、缩放/复位、子路径
// 更新、多根数组、错误回路、纯客户端交互（零 fire）。
// 用法：node test-trace.js。浏览器行为验收另做（www/ui-trace-acc.html）。

function classList(el) { return el.className ? el.className.split(/\s+/) : []; }
function elMatches(el, sel) {
  if (!sel.startsWith('.') && !sel.startsWith('#')) return false;
  const want = sel.slice(1);
  return classList(el).includes(want) || el.dataset.uiPath === want;
}
function makeEl(tag) {
  const el = {
    tagName: tag.toUpperCase(), children: [], className: '', dataset: {}, style: {},
    textContent: '', _parent: null, attrs: {}, type: undefined, listeners: {},
    appendChild(c) { c._parent = el; el.children.push(c); return c; },
    append(...cs) { for (const c of cs) { c._parent = el; el.children.push(c); } },
    remove() { if (el._parent) el._parent.children = el._parent.children.filter(x => x !== el); },
    replaceChildren(...cs) { el.children = []; for (const c of cs) { c._parent = el; el.children.push(c); } },
    addEventListener(t, f) { (el.listeners[t] ??= []).push(f); },
    setAttribute(k, v) { el.attrs[k] = String(v); if (k === 'class') el.className = String(v); },
    getAttribute(k) { return el.attrs[k]; },
    dispatchEv(t, ev) { ev.target = ev.target ?? el; let cur = el; while (cur) { for (const f of cur.listeners[t] || []) f(ev); cur = cur._parent; } },
    closest(sel) { let cur = el; while (cur) { if (elMatches(cur, sel)) return cur; cur = cur._parent; } return null; },
    classList: {
      add(c) { if (!classList(el).includes(c)) el.className = (el.className ? el.className + ' ' : '') + c; },
      contains(c) { return classList(el).includes(c); },
    },
    head: undefined,
  };
  if (tag === 'button') el.type = 'button';
  return el;
}
const rafQ = [];
globalThis.document = {
  createElement(tag) { return makeEl(tag); },
  createElementNS(ns, tag) { return makeEl(tag); },
  head: makeEl('head'),
};
globalThis.requestAnimationFrame = (cb) => { rafQ.push(cb); return rafQ.length; };
function flushRaf() { while (rafQ.length) { const cb = rafQ.shift(); cb(0); } }

const { createUiRuntime, registerDomRenderers, injectUiStyles } = await import('./www/ui-runtime.js');

let fails = 0;
const ok = (cond, label, d) => {
  if (cond) console.log(`  ok  ${label}`);
  else { fails++; console.error(`FAIL  ${label}${d !== undefined ? ' :: ' + JSON.stringify(d) : ''}`); }
};

const fires = [];
const errors = [];
const root = makeEl('div');
const runtime = createUiRuntime(root, { fire: (...a) => fires.push(a), onError: (m) => errors.push(m) });
registerDomRenderers(runtime);
injectUiStyles();

const spanTree = {
  name: 'http.request', start_ms: 0, dur_ms: 240,
  children: [
    { name: 'db.query', start_ms: 10, dur_ms: 120, children: [
      { name: 'SELECT', start_ms: 15, dur_ms: 40, children: [] },
      { name: 'index scan', start_ms: 60, dur_ms: 60, children: [] },
    ] },
    { name: 'cache.get', start_ms: 140, dur_ms: 30, children: [] },
    { name: 'serialize', start_ms: 175, dur_ms: 60, children: [] },
  ],
};
runtime.applyOps([{ op: 'create', path: '.tr', type: 'Trace', props: { bind: '/trace' }, events: [] }]);
const t = root.children[0];
const st = t._trace;
ok(st && t.children.length === 0, 'create: empty until data');
flushRaf();
ok(t.children.length === 1 && classList(t.children[0]).includes('rui-trace-empty'), 'empty state');

runtime.applyOps([{ op: 'model', sub: 'set', path: '/trace', value: spanTree }]);
flushRaf();
const curRows = () => t.children[0].children;   // 每次读当前渲染的节点（重渲染换节点）
const labels = () => curRows().map(r => r.children[0].children[1].textContent);
const barOf = (name) => {
  const r = curRows().find(r => r.children[0].children[1].textContent === name);
  return r.children[1].children[0];
};
ok(curRows().length === 6, 'full tree: 6 visible rows (expanded by default)', labels());
ok(labels().join(',') === 'http.request,db.query,SELECT,index scan,cache.get,serialize', 'pre-order span order', labels());
ok(Math.abs(parseFloat(barOf('http.request').style.left) - 0) < 0.01
  && Math.abs(parseFloat(barOf('http.request').style.width) - 100) < 0.01, 'root bar spans full domain', barOf('http.request').style);
ok(Math.abs(parseFloat(barOf('db.query').style.left) - (10 / 240 * 100)) < 0.01
  && Math.abs(parseFloat(barOf('db.query').style.width) - (120 / 240 * 100)) < 0.01, 'child bar geometry', barOf('db.query').style);
ok(curRows()[2].children[0].style.paddingLeft === '24px' && curRows()[1].children[0].style.paddingLeft === '12px', 'depth indentation', curRows()[2].children[0].style.paddingLeft);

// 折叠 db.query → 行数减、其余条对齐不变
const dbRow = curRows()[1];
dbRow.children[0].dispatchEv('click', {});   // 名字区点击
flushRaf();
ok(t.children[0].children.length === 4, 'fold hides descendant rows', t.children[0].children.length);
const rows2 = t.children[0].children;
const q = rows2.find(r => r.children[0].children[1].textContent === 'db.query');
ok(Math.abs(parseFloat(q.children[1].children[0].style.left) - (10 / 240 * 100)) < 0.01, 'fold keeps bar alignment (full-tree domain)', q.children[1].children[0].style.left);
// 展开回去
rows2.find(r => r.children[0].children[1].textContent === 'db.query').children[0].dispatchEv('click', {});
flushRaf();
ok(t.children[0].children.length === 6, 'unfold restores rows');

// 缩放：点 db.query 条 → 域 = [10,130]
barOf('db.query').dispatchEv('click', {});
flushRaf();
ok(Math.abs(parseFloat(barOf('db.query').style.left) - 0) < 0.01
  && Math.abs(parseFloat(barOf('db.query').style.width) - 100) < 0.01, 'zoom: clicked span fills track');
ok(Math.abs(parseFloat(barOf('http.request').style.left) - (-10 / 120 * 100)) < 0.01, 'zoom: root bar shifted left (clipped)', barOf('http.request').style.left);
ok(Math.abs(parseFloat(barOf('SELECT').style.left) - (5 / 120 * 100)) < 0.01
  && Math.abs(parseFloat(barOf('SELECT').style.width) - (40 / 120 * 100)) < 0.01, 'zoom: child geometry rescaled', barOf('SELECT').style);
// 再点同条 → 复位
barOf('db.query').dispatchEv('click', {});
flushRaf();
ok(Math.abs(parseFloat(barOf('db.query').style.width) - (120 / 240 * 100)) < 0.01, 'second click resets zoom');
// 缩放后点轨道空白 → 复位
barOf('db.query').dispatchEv('click', {});
flushRaf();
barOf('cache.get').closest('.rui-trace-track').dispatchEv('click', {});
flushRaf();
ok(Math.abs(parseFloat(barOf('db.query').style.width) - (120 / 240 * 100)) < 0.01, 'track background click resets zoom');

// 子路径更新：cache.get（children[1]）时长 30 → 90（viewDelta 合成），
// 域仍是 [0,240]（根 span 结尾 240 覆盖），宽度随之 12.5% → 37.5%
runtime.applyOps([{ op: 'model', sub: 'set', path: '/trace/children/1/dur_ms', value: 90 }]);
flushRaf();
ok(Math.abs(parseFloat(barOf('cache.get').style.width) - (90 / 240 * 100)) < 0.01, 'subpath update rescales bar', barOf('cache.get').style.width);

// 多根数组
runtime.applyOps([{ op: 'create', path: '.tr2', type: 'Trace', props: { bind: '/mt' }, events: [] }]);
const t2 = root.children[1]._trace;
runtime.applyOps([{ op: 'model', sub: 'set', path: '/mt', value: [{ name: 'a', start_ms: 0, dur_ms: 10 }, { name: 'b', start_ms: 5, dur_ms: 10 }] }]);
flushRaf();
ok(t2.el.children[0].children.length === 2, 'array root: multiple top-level spans');
ok(Math.abs(parseFloat(t2.el.children[0].children[1].children[1].children[0].style.left) - (5 / 15 * 100)) < 0.01, 'array root domain spans both roots ([0,15] -> 33.3%)');

// 无效根 → onError
runtime.applyOps([{ op: 'model', sub: 'set', path: '/trace', value: 'nope' }]);
ok(errors.length === 1 && errors[0].includes('must be a span object or array'), 'non-object root -> onError', errors);
// 无效 span（缺 timing）防御性跳过
runtime.applyOps([{ op: 'model', sub: 'set', path: '/mt', value: [{ name: 'ok', start_ms: 0, dur_ms: 5 }, { name: 'bad' }] }]);
flushRaf();
ok(t2.el.children[0].children.length === 1, 'span without timing is skipped');

// 纯客户端：折叠/缩放零 fire
ok(fires.length === 0, 'fold/zoom never fires to Tcl', fires);

const done = fails === 0;
console.log(done ? 'SMOKE OK' : `SMOKE FAILED (${fails})`);
process.exit(done ? 0 : 1);
