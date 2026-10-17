// ui-runtime Log 渲染器的 node 冒烟：迷你 DOM shim + 同步 rAF。
// 校验：虚拟窗口有界、游标追加原位写、follow 钉底 / 上滚暂停 /
// 回底恢复、lineclick 委托、级别类名（含非法 level 落 info）、
// 标量行值、错误回路。用法：node test-log.js（纯 JS，不需要 wasm）。
// 浏览器行为验收另做（www/ui-log-acc.html）。

// ── 迷你 DOM（同 test-table.js，外加浏览器级滚动钳制模拟）────
function classList(el) { return el.className ? el.className.split(/\s+/) : []; }
function elMatches(el, sel) {
  if (!sel.startsWith('.') && !sel.startsWith('#')) return false;
  const want = sel.slice(1);
  return classList(el).includes(want) || el.dataset.uiPath === want;
}
function makeEl(tag) {
  const el = {
    tagName: tag.toUpperCase(), children: [], className: '', dataset: {}, style: {},
    textContent: '', _parent: null, clientHeight: 280,
    type: undefined, listeners: {},
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
    head: undefined,
  };
  if (tag === 'button') el.type = 'button';
  // 浏览器语义：scrollHeight = 内容高（spacer 的 height 样式）；
  // scrollTop 写入时钳到 [0, scrollHeight - clientHeight]（真实 DOM
  // 的钳制——follow 钉底靠它落到可视尾窗）。
  Object.defineProperty(el, 'scrollHeight', {
    get() {
      const sp = el.children[0];
      return Math.max(0, Math.round(parseFloat(sp?.style?.height) || 0));
    },
  });
  let _scrollTop = 0;
  Object.defineProperty(el, 'scrollTop', {
    get() { return _scrollTop; },
    set(v) {
      _scrollTop = Math.max(0, Math.min(v, Math.max(0, el.scrollHeight - el.clientHeight)));
    },
  });
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

// 创建（events 含 lineclick → 挂行委托）
runtime.applyOps([{
  op: 'create', path: '.l', type: 'Log',
  props: { bind: '/deploylog' },
  events: ['lineclick'],
}]);
const t = root.children[0];
const st = t._log;
ok(st && st.n === 0 && st.rows.length === 0, 'create: empty until data');
ok(st.following === true, 'create: starts following');
ok(t.listeners.click?.length === 1, 'lineclick delegate attached (events carried it)');
injectUiStyles();
ok(document.head.children.length === 1, 'styles injectable');

// 游标追加 300 行：原位缓存 + 窗口有界 + follow 钉底
for (let i = 0; i < 300; i++) {
  runtime.applyOps([{
    op: 'model', sub: 'set', path: `/deploylog/${i}`,
    value: { ts: `00:0${Math.floor(i / 60)}:${i % 60}`, level: ['info', 'warn', 'error', 'debug'][i % 4], text: `step ${i}: ok` },
  }]);
}
flushRaf();
ok(st.rows.length === 300 && st.rows[299].text === 'step 299: ok', 'cursor feed fills line cache');
ok(st.n === 300, 'line count tracks cursor');
ok(st.pool.size <= 13 + 16 + 1, 'virtual window bounded (viewport+buffer)', st.pool.size);
ok(st.following === true, 'appends keep following');
ok(t.scrollTop === t.scrollHeight - t.clientHeight, 'follow pins scrollTop to bottom (clamped)');
const tailIdx = Math.max(...st.pool.keys());
ok(tailIdx === 299, 'tail line rendered while following', tailIdx);
console.log(`  ..  pool.size=${st.pool.size} n=${st.n}`);

// 级别类名 + 文本（textContent，不拼 HTML）
const tail = st.pool.get(299);
ok(tail.className === 'rui-log-line rui-lv-debug', 'tail line level class (299%4=debug)', tail.className);
ok(tail._text.textContent === 'step 299: ok', 'line text set via textContent');

// 单行重写（同 idx 再 set）：节点身份保持，原位更新
const before = st.pool.get(299);
runtime.applyOps([{ op: 'model', sub: 'set', path: '/deploylog/299', value: { level: 'error', text: 'step 299: FAILED' } }]);
ok(st.pool.get(299) === before, 'single-line update reuses node');
ok(before._text.textContent === 'step 299: FAILED' && before.className.includes('rui-lv-error'), 'rewritten in place with new level');

// 上滚 → 暂停跟随；窗口钉在滚动位置
t.scrollTop = 100 * 22;
t.dispatchEv('scroll', {});
flushRaf();
ok(st.following === false, 'scroll up pauses follow');
ok(st.pool.has(100), 'paused window shows scrolled-at lines', [...st.pool.keys()]);

// 暂停期间追加：不钉底、不抢滚动位置
const scAt = t.scrollTop;
for (let i = 300; i < 320; i++) {
  runtime.applyOps([{ op: 'model', sub: 'set', path: `/deploylog/${i}`, value: { level: 'info', text: `late ${i}` } }]);
}
ok(t.scrollTop === scAt, 'paused: appends do not move scroll');
ok(st.n === 320, 'paused: cache still grows');
flushRaf();

// lineclick 委托（暂停窗口仍显示 100 行；冒泡：行 → spacer → 容器）
fired.length = 0;
st.pool.get(100).dispatchEv('click', { target: st.pool.get(100) });
ok(fired[0]?.[0] === '.l' && fired[0]?.[1] === 'lineclick' && String(fired[0]?.[2]) === '100', 'line body fires lineclick with detail', fired[fired.length - 1]);

// 回底 → 恢复跟随
t.scrollTop = t.scrollHeight - t.clientHeight;
t.dispatchEv('scroll', {});
flushRaf();
ok(st.following === true, 'scroll to bottom resumes follow');
const tail2 = Math.max(...st.pool.keys());
ok(tail2 === 319, 'resumed window covers tail', tail2);

// 标量行值 → info + 文本；非法 level 落 info（类名注入边界）
runtime.applyOps([{ op: 'model', sub: 'set', path: '/deploylog/319', value: 'plain text line' }]);
ok(st.pool.get(319).className === 'rui-log-line rui-lv-info' && st.pool.get(319)._text.textContent === 'plain text line', 'scalar line value renders as info');
runtime.applyOps([{ op: 'model', sub: 'set', path: '/deploylog/318', value: { level: 'x y;drop', text: 'evil' } }]);
ok(st.pool.get(318).className === 'rui-log-line rui-lv-info', 'malicious level falls back to info class', st.pool.get(318).className);

// unset：置洞渲染空行
runtime.applyOps([{ op: 'model', sub: 'unset', path: '/deploylog/317' }]);
ok(st.rows[317] === undefined, 'unset leaves hole in cache');
ok(st.pool.get(317)?._text.textContent === '', 'hole renders empty line');

// 非 lineclick events → 不挂委托
runtime.applyOps([{ op: 'create', path: '.quiet', type: 'Log', props: { bind: '/q' }, events: [] }]);
const q = root.children[1];
ok(q._log && !q.listeners.click, 'no delegate when events empty');

// 错误回路：非数组全量喂 → onError（不静默）
runtime.applyOps([{ op: 'create', path: '.bad', type: 'Log', props: { bind: '/x' } }]);
runtime.applyOps([{ op: 'model', sub: 'set', path: '/x', value: 'not-an-array' }]);
ok(errors.length === 1 && errors[0].includes('must be a JSON array'), 'non-array lines -> onError', errors);

// 全量 set（初始数组形态）
runtime.applyOps([{ op: 'model', sub: 'set', path: '/q', value: [{ level: 'warn', text: 'a' }, 'plain', { text: 'no-level' }] }]);
ok(q._log.n === 3, 'full-array init');
ok(q._log.pool.get(0).className.includes('rui-lv-warn') && q._log.pool.get(1)._text.textContent === 'plain' && q._log.pool.get(2).className.includes('rui-lv-info'), 'mixed line shapes render', [q._log.pool.get(0).className, q._log.pool.get(1)._text.textContent, q._log.pool.get(2).className]);

const done = fails === 0;
console.log(done ? 'SMOKE OK' : `SMOKE FAILED (${fails})`);
process.exit(done ? 0 : 1);
