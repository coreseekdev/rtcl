// ui-runtime Chart 渲染器的 node 冒烟：迷你 DOM shim + 同步 rAF。
// 校验：line/bar/point/area 子集、全量/增量喂数、rAF 合并重绘、
// spec 错误回路（onError）、spec 更新重建。
// 用法：node test-chart.js。浏览器行为验收另做（www/ui-chart-acc.html）。

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
    closest(sel) { let cur = el; while (cur) { if (elMatches(cur, sel)) return cur; cur = cur._parent; } return null; },
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

const errors = [];
const root = makeEl('div');
const runtime = createUiRuntime(root, { fire() {}, onError: (m) => errors.push(m) });
registerDomRenderers(runtime);
injectUiStyles();

const lineSpec = {
  data: { name: '/cpu' },
  mark: 'line',
  encoding: { x: { field: 't' }, y: { field: 'v', type: 'quantitative' } },
};
runtime.applyOps([{ op: 'create', path: '.c', type: 'Chart', props: { vegalite: lineSpec }, events: [] }]);
const c = root.children[0];
const st = c._chart;
ok(st && st.plot, 'create: chart state + plot');
flushRaf();
ok(st.plot.children.length === 1 && classList(st.plot.children[0]).includes('rui-chart-empty'), 'empty state before data', st.plot.children.length);

// 全量喂 3 点（数组一次 set）
runtime.applyOps([{ op: 'model', sub: 'set', path: '/cpu', value: [{ t: 'a', v: 1 }, { t: 'b', v: 3 }, { t: 'c', v: 2 }] }]);
flushRaf();
const kids = st.plot.children;
const cls = (el) => classList(el);
const svg = kids.find(e => cls(e).includes('rui-chart-svg'));
const line = svg?.children.find(e => cls(e).includes('rui-chart-line'));
const pairs = (line?.getAttribute('points') || '').split(' ').filter(Boolean);
ok(kids.filter(e => cls(e).includes('rui-chart-grid')).length === 5, 'y gridlines = 5');
ok(kids.filter(e => cls(e).includes('rui-chart-ytick')).length === 5, 'y tick labels = 5');
ok(kids.filter(e => cls(e).includes('rui-chart-xtick')).length === 3, 'x tick labels = 3 (all fit)', kids.filter(e => cls(e).includes('rui-chart-xtick')).length);
ok(pairs.length === 3, 'line has 3 points', pairs);
const yOf = (p) => Number(p.split(',')[1]);
ok(yOf(pairs[1]) < yOf(pairs[0]) && yOf(pairs[2]) > yOf(pairs[1]), 'y axis inverted (higher value = smaller %)', pairs);
ok(kids.indexOf(svg) > 0 && kids.filter(e => cls(e).includes('rui-chart-xtick')).every(e => String(e.style.left).endsWith('%')), 'labels positioned in %');

// 增量 append（游标：一次一行）→ rAF 合并
runtime.applyOps([{ op: 'model', sub: 'set', path: '/cpu/3', value: { t: 'd', v: 5 } }]);
runtime.applyOps([{ op: 'model', sub: 'set', path: '/cpu/4', value: { t: 'e', v: 0 } }]);
flushRaf();
const line2 = st.plot.children.find(e => cls(e).includes('rui-chart-svg'))?.children
  .find(e => cls(e).includes('rui-chart-line'));
ok((line2?.getAttribute('points') || '').split(' ').filter(Boolean).length === 5, 'incremental appends extend the line', line2?.getAttribute('points'));

// x 标签采样上限（20 点 → ≤7 个标签）
const many = [];
for (let i = 0; i < 20; i++) many.push({ t: 't' + i, v: i * 2 });
runtime.applyOps([{ op: 'model', sub: 'set', path: '/cpu', value: many }]);
flushRaf();
const xlabels = st.plot.children.filter(e => cls(e).includes('rui-chart-xtick'));
ok(xlabels.length <= 7 && xlabels.length >= 3, 'x labels sampled (<=7)', xlabels.length);

// bar mark
const barSpec = {
  data: { name: '/bars' }, mark: 'bar',
  encoding: { x: { field: 'k' }, y: { field: 'n' } },
};
runtime.applyOps([{ op: 'create', path: '.b', type: 'Chart', props: { vegalite: barSpec }, events: [] }]);
const b = root.children[1]._chart;
runtime.applyOps([{ op: 'model', sub: 'set', path: '/bars', value: [{ k: 'a', n: 2 }, { k: 'b', n: 5 }, { k: 'c', n: 1 }] }]);
flushRaf();
const bars = b.plot.children.filter(e => cls(e).includes('rui-chart-bar'));
ok(bars.length === 3, 'bar mark: one div per point', bars.length);
const tallest = bars.map(e => parseFloat(e.style.height)).sort((x, y) => y - x)[0];
ok(tallest > 0 && bars.every(e => parseFloat(e.style.height) > 0), 'bar heights positive', bars.map(e => e.style.height));

// point mark
runtime.applyOps([{
  op: 'create', path: '.p', type: 'Chart',
  props: { vegalite: { data: { name: '/pts' }, mark: 'point', encoding: { x: { field: 'x' }, y: { field: 'y' } } } },
  events: [],
}]);
const p = root.children[2]._chart;
runtime.applyOps([{ op: 'model', sub: 'set', path: '/pts', value: [{ x: 'a', y: 1 }, { x: 'b', y: 2 }] }]);
flushRaf();
ok(p.plot.children.filter(e => cls(e).includes('rui-chart-dot')).length === 2, 'point mark dots');

// area mark
runtime.applyOps([{
  op: 'create', path: '.a', type: 'Chart',
  props: { vegalite: { data: { name: '/ar' }, mark: 'area', encoding: { x: { field: 'x' }, y: { field: 'y' } } } },
  events: [],
}]);
const a = root.children[3]._chart;
runtime.applyOps([{ op: 'model', sub: 'set', path: '/ar', value: [{ x: 'a', y: 1 }, { x: 'b', y: 2 }] }]);
flushRaf();
const areaSvg = a.plot.children.find(e => cls(e).includes('rui-chart-svg'));
ok(areaSvg?.children.some(e => cls(e).includes('rui-chart-area')), 'area mark polygon');

// spec 结构错误 → onError（不静默）
runtime.applyOps([{
  op: 'create', path: '.bad', type: 'Chart',
  props: { vegalite: { mark: 'line', encoding: { x: { field: 't' } } } },
  events: [],
}]);
ok(errors.length === 1 && errors[0].includes('encoding.x.field and spec.encoding.y.field'), 'missing encoding field -> onError', errors);
runtime.applyOps([{
  op: 'create', path: '.bad2', type: 'Chart',
  props: { vegalite: { mark: 'pie', encoding: { x: { field: 't' }, y: { field: 'v' } } } },
  events: [],
}]);
ok(errors.length === 2 && errors[1].includes('unsupported mark'), 'unsupported mark -> onError', errors);

// spec 更新（mark line → bar）经 set op 重建图形
runtime.applyOps([{ op: 'set', path: '.c', props: { vegalite: barSpec, bind: undefined }, events: [] }]);
flushRaf();
// 注意：.c 的数据路径变为 /bars（barSpec），先喂数据
runtime.applyOps([{ op: 'model', sub: 'set', path: '/bars', value: [{ k: 'x', n: 1 }] }]);
flushRaf();
ok(st.plot.children.filter(e => cls(e).includes('rui-chart-bar')).length === 1, 'spec update via set op re-renders with new mark');

const done = fails === 0;
console.log(done ? 'SMOKE OK' : `SMOKE FAILED (${fails})`);
process.exit(done ? 0 : 1);
