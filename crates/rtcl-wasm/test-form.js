// ui-runtime Form 渲染器的 node 冒烟：迷你 DOM shim。
// 校验：schema 子集控件、模型初始值、变更才回写（击键不触 Tcl）、
// 类型化提交、required 校验挡提交（标红+提示+不回灌）、submit 回灌
// commit、外部模型更新回推控件、schema 结构错误 onError。
// 用法：node test-form.js。浏览器行为验收另做（www/ui-form-acc.html）。

function classList(el) { return el.className ? el.className.split(/\s+/) : []; }
function elMatches(el, sel) {
  if (!sel.startsWith('.') && !sel.startsWith('#')) return false;
  const want = sel.slice(1);
  return classList(el).includes(want) || el.dataset.uiPath === want;
}
function makeEl(tag) {
  const el = {
    tagName: tag.toUpperCase(), children: [], className: '', dataset: {}, style: {},
    textContent: '', value: '', checked: false, _parent: null, type: undefined, listeners: {},
    appendChild(c) { c._parent = el; el.children.push(c); return c; },
    append(...cs) { for (const c of cs) { c._parent = el; el.children.push(c); } },
    remove() { if (el._parent) el._parent.children = el._parent.children.filter(x => x !== el); },
    replaceChildren(...cs) { el.children = []; for (const c of cs) { c._parent = el; el.children.push(c); } },
    addEventListener(t, f) { (el.listeners[t] ??= []).push(f); },
    dispatchEv(t, ev) { ev = ev ?? {}; ev.target = ev.target ?? el; let cur = el; while (cur) { for (const f of cur.listeners[t] || []) f(ev); cur = cur._parent; } },
    closest(sel) { let cur = el; while (cur) { if (elMatches(cur, sel)) return cur; cur = cur._parent; } return null; },
    classList: {
      add(c) { if (!classList(el).includes(c)) el.className = (el.className ? el.className + ' ' : '') + c; },
      remove(c) { el.className = classList(el).filter(x => x !== c).join(' '); },
      contains(c) { return classList(el).includes(c); },
      toggle(c, on) { if (on) el.classList.add(c); else el.classList.remove(c); },
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
const commits = [];
const errors = [];
const root = makeEl('div');
let runtime = null;
runtime = createUiRuntime(root, {
  fire: (...a) => fired.push(a),
  // 模拟宿主回路（ui.html 同款）：提交 → 回灌 Tcl → Tcl 发模型 op 回来
  commitModel: (p, v) => { commits.push([p, v]); runtime.applyOps([{ op: 'model', sub: 'set', path: p, value: v }]); },
  onError: (m) => errors.push(m),
});
registerDomRenderers(runtime);
injectUiStyles();

const schema = {
  type: 'object',
  properties: {
    email: { type: 'string', title: 'Email' },
    age: { type: 'integer' },
    plan: { enum: ['free', 'pro'] },
    urgent: { type: 'boolean' },
  },
  required: ['email', 'urgent'],
};
runtime.applyOps([{ op: 'create', path: '.f', type: 'Form', props: { bind: '/form', schema }, events: ['commit'] }]);
const el = root.children[0];
const st = el._form;
ok(st && st.inputs.size === 4, 'create: four fields from properties');
const inputs = () => st.inputs;
ok(inputs().get('email').input.tagName === 'INPUT' && inputs().get('email').input.type === 'text', 'string -> text input');
ok(inputs().get('age').input.type === 'number', 'integer -> number input');
ok(inputs().get('plan').input.tagName === 'SELECT' && inputs().get('plan').input.children.length === 2, 'enum -> select with options');
ok(inputs().get('urgent').input.type === 'checkbox', 'boolean -> checkbox');
ok(inputs().get('email').input.closest('.rui-form-field').children[0].textContent === 'Email *', 'title label + required star', inputs().get('email').input.closest('.rui-form-field').children[0].textContent);

// 模型初始值灌入（基路径全量 set）
runtime.applyOps([{ op: 'model', sub: 'set', path: '/form', value: { email: 'a@b', age: 3, plan: 'pro', urgent: true } }]);
ok(inputs().get('email').input.value === 'a@b' && inputs().get('age').input.value === '3', 'initial values from model');
ok(inputs().get('plan').input.value === 'pro' && inputs().get('urgent').input.checked === true, 'select/checkbox initial');

// 击键不触 Tcl：input 事件不产生提交
inputs().get('email').input.value = 'typed@x';
inputs().get('email').input.dispatchEv('input', {});
ok(commits.length === 0, 'keystroke (input event) does not commit', commits);

// change 才回写；类型化
inputs().get('email').input.dispatchEv('change', {});
inputs().get('age').input.value = '42';
inputs().get('age').input.dispatchEv('change', {});
inputs().get('urgent').input.checked = false;
inputs().get('urgent').input.dispatchEv('change', {});
inputs().get('plan').input.value = 'free';
inputs().get('plan').input.dispatchEv('change', {});
ok(JSON.stringify(commits) === JSON.stringify([
  ['/form/email', 'typed@x'], ['/form/age', 42], ['/form/urgent', false], ['/form/plan', 'free'],
]), 'typed commits on change (string/number/bool/enum)', commits);

// 外部模型更新回推控件
runtime.applyOps([{ op: 'model', sub: 'set', path: '/form/email', value: 'external@z' }]);
ok(inputs().get('email').input.value === 'external@z', 'external field update pushes into input');

// 提交：required 缺失（email 清空 + urgent 未勾）→ 标红 + 提示 + 不回灌
inputs().get('email').input.value = '';
inputs().get('email').input.dispatchEv('change', {});
inputs().get('urgent').input.checked = false;
fired.length = 0;
st.err; // eslint noop
const submitBtn = el.children.at(-1);
submitBtn.dispatchEv('click', {});
ok(fired.length === 0, 'invalid submit does not fire commit', fired);
ok(inputs().get('email').input.classList.contains('rui-form-bad') && inputs().get('urgent').input.classList.contains('rui-form-bad'), 'missing fields marked');
ok(el.children.at(-2).textContent === 'missing required: email, urgent' && el.children.at(-2).classList.contains('rui-form-error-show'), 'error message shown', el.children.at(-2).textContent);

// 填好 → 提交通过：回灌 commit、错误清除
inputs().get('email').input.value = 'ok@y';
inputs().get('email').input.dispatchEv('change', {});
inputs().get('urgent').input.checked = true;
inputs().get('urgent').input.dispatchEv('change', {});
fired.length = 0;
submitBtn.dispatchEv('click', {});
ok(fired.length === 1 && fired[0][0] === '.f' && fired[0][1] === 'commit', 'valid submit fires commit once', fired);
ok(el.children.at(-2).textContent === '' && !inputs().get('email').input.classList.contains('rui-form-bad'), 'errors cleared after valid submit');

// Enter 提交（文本输入 keydown）
fired.length = 0;
inputs().get('email').input.dispatchEv('keydown', { key: 'Enter' });
ok(fired.length === 1 && fired[0][1] === 'commit', 'Enter in text field submits', fired);

// schema 结构错误 → onError
runtime.applyOps([{ op: 'create', path: '.bad', type: 'Form', props: { bind: '/f2', schema: { type: 'object' } }, events: [] }]);
ok(errors.length === 1 && errors[0].includes('properties object'), 'schema without properties -> onError', errors);
runtime.applyOps([{ op: 'create', path: '.bad2', type: 'Form', props: { bind: '/f3', schema: { type: 'object', properties: { x: { type: 'array' } } } }, events: [] }]);
ok(errors.length === 2 && errors[1].includes('unsupported type "array"'), 'unsupported type -> onError', errors);

// 无 -path：commit 不写模型，但 submit 仍回灌
runtime.applyOps([{ op: 'create', path: '.np', type: 'Form', props: { schema: { type: 'object', properties: { q: { type: 'string' } } } }, events: ['commit'] }]);
const np = root.children.at(-1)._form;
const nCommit = commits.length;
np.inputs.get('q').input.value = 'v';
np.inputs.get('q').input.dispatchEv('change', {});
ok(commits.length === nCommit, 'no -path: values stay local', commits);
fired.length = 0;
root.children.at(-1).children.at(-1).dispatchEv('click', {});
ok(fired.length === 1 && fired[0][0] === '.np' && fired[0][1] === 'commit', 'no -path: submit still fires');

const done = fails === 0;
console.log(done ? 'SMOKE OK' : `SMOKE FAILED (${fails})`);
process.exit(done ? 0 : 1);
