// ui-runtime Confirm 渲染器的 node 冒烟：迷你 DOM shim。
// 校验：danger/预览（字符串=文字、对象=缩进 JSON）、ok/cancel 回灌
// 恰好一次、决定后禁用、configure 重新武装、无预览裸按钮。
// 用法：node test-confirm.js。浏览器行为验收另做（www/ui-confirm-acc.html）。

function classList(el) { return el.className ? el.className.split(/\s+/) : []; }
function elMatches(el, sel) {
  if (!sel.startsWith('.') && !sel.startsWith('#')) return false;
  const want = sel.slice(1);
  return classList(el).includes(want) || el.dataset.uiPath === want;
}
function makeEl(tag) {
  const el = {
    tagName: tag.toUpperCase(), children: [], className: '', dataset: {}, style: {},
    textContent: '', _parent: null, type: undefined, disabled: false, listeners: {},
    appendChild(c) { c._parent = el; el.children.push(c); return c; },
    append(...cs) { for (const c of cs) { c._parent = el; el.children.push(c); } },
    remove() { if (el._parent) el._parent.children = el._parent.children.filter(x => x !== el); },
    replaceChildren(...cs) { el.children = []; for (const c of cs) { c._parent = el; el.children.push(c); } },
    addEventListener(t, f) { (el.listeners[t] ??= []).push(f); },
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
globalThis.document = { createElement: makeEl, createElementNS: () => makeEl('x'), head: makeEl('head') };
globalThis.requestAnimationFrame = (cb) => { cb(0); return 0; };

const { createUiRuntime, registerDomRenderers, injectUiStyles } = await import('./www/ui-runtime.js');

let fails = 0;
const ok = (cond, label, d) => {
  if (cond) console.log(`  ok  ${label}`);
  else { fails++; console.error(`FAIL  ${label}${d !== undefined ? ' :: ' + JSON.stringify(d) : ''}`); }
};

const fired = [];
const errors = [];
const root = makeEl('div');
const runtime = createUiRuntime(root, { fire: (...a) => fired.push(a), onError: (m) => errors.push(m) });
registerDomRenderers(runtime);
injectUiStyles();

const card = () => {
  const el = root.children[0];
  const st = el._confirm;
  return { el, st, okBtn: el.children.at(-1).children[0], noBtn: el.children.at(-1).children[1] };
};

runtime.applyOps([{
  op: 'create', path: '.c', type: 'Confirm',
  props: { danger: true, preview: { issue: 'SENTRY-42', affected: 3 } },
  events: ['ok'],
}]);
{
  const { el, okBtn, noBtn } = card();
  const cls = el.className.split(/\s+/);
  ok(cls.includes('rui-confirm') && cls.includes('rui-confirm-danger'), 'danger class');
  ok(el.children[0].tagName === 'PRE' && el.children[0].textContent.includes('"issue": "SENTRY-42"'), 'object preview = indented JSON', el.children[0].textContent);
  ok(okBtn.textContent === 'Confirm' && noBtn.textContent === 'Cancel', 'buttons labeled');
  ok(okBtn.disabled === false, 'buttons armed');
}

// 点 Confirm → 恰好一次 ok 回灌 + 禁用
{
  const { okBtn } = card();
  okBtn.dispatchEv('click', {});
  ok(fired.length === 1 && fired[0][0] === '.c' && fired[0][1] === 'ok', 'ok fired once', fired);
  const after = card();   // 重渲染换节点：重新取
  ok(after.el._confirm.decided === true && after.okBtn.disabled === true && after.noBtn.disabled === true, 'decided disables both buttons');
  ok(after.el.className.includes('rui-confirm-done'), 'done styling');
  after.okBtn.dispatchEv('click', {});   // 二击：真 DOM disabled 已拦；decided 兜底
  ok(fired.length === 1, 'second click does not re-fire', fired);
}

// configure 更新 preview → 重新武装（新问题）
runtime.applyOps([{ op: 'set', path: '.c', props: { preview: 'Delete 3 issues?' }, events: [] }]);
{
  const { el, okBtn, noBtn } = card();
  ok(el.children[0].className.includes('rui-confirm-msg'), 'string preview = message div');
  ok(el.children[0].textContent === 'Delete 3 issues?', 'message text');
  ok(el._confirm.decided === false && okBtn.disabled === false && noBtn.disabled === false, 're-armed after configure');
  const dangerStill = el.className.includes('rui-confirm-danger');
  ok(dangerStill, 'danger persists when not reconfigured');
  okBtn.dispatchEv('click', {});
  ok(fired.length === 2 && fired[1][1] === 'ok', 'second round fires again', fired);
}

// Cancel 路径 + 无预览
runtime.applyOps([{ op: 'create', path: '.c2', type: 'Confirm', props: {}, events: [] }]);
{
  const el2 = root.children[1];
  const okBtn = el2.children[0].children[0];
  ok(el2.children.length === 1, 'no preview -> only the button row');
  ok(!el2.className.includes('rui-confirm-danger'), 'no danger by default');
  el2.children[0].children[1].dispatchEv('click', {});   // Cancel
  const last = fired[fired.length - 1];
  ok(last[0] === '.c2' && last[1] === 'cancel', 'cancel fires', last);
  ok(el2._confirm.decided === true, 'cancel also decides');
  okBtn.dispatchEv('click', {});
  ok(fired.filter(f => f[0] === '.c2').length === 1, 'no ok after cancel', fired);
}

ok(errors.length === 0, 'no renderer errors', errors);

const done = fails === 0;
console.log(done ? 'SMOKE OK' : `SMOKE FAILED (${fails})`);
process.exit(done ? 0 : 1);
