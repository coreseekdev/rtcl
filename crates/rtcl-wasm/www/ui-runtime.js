// ui-runtime.js — rtcl ui::* op 协议的 JS 运行时（契约 docs/UI-DSL.md §3）
//
// 三件事：op 解释（create/set/delete/model）、widget/模型存储、渲染器
// 注册表。渲染器只产出带 class 的 DOM，样式全部在外部 CSS（headless
// 纪律：结构/样式分离，换主题不动渲染器）。

export function createUiRuntime(root, opts = {}) {
  const fire = opts.fire ?? (() => {});
  const commitModel = opts.commitModel ?? (() => {});
  const onError = opts.onError ?? ((m) => console.error('[ui]', m));

  const widgets = new Map();    // path -> {el, type, renderer, props}
  const model = new Map();      // model path -> JS 值
  const subs = new Map();       // model path -> Map(widgetPath -> callback)
  const renderers = new Map();  // type -> {create, update?, container?, dispose?}

  function registerRenderer(type, r) { renderers.set(type, r); }

  function parentPath(path) {
    const i = path.lastIndexOf('.');
    return i <= 0 ? '' : path.slice(0, i);
  }

  function modelSet(path, value) {
    model.set(path, value);
    const s = subs.get(path);
    if (s) for (const cb of s.values()) cb(value);
  }

  function modelUnset(path) {
    model.delete(path);
    const s = subs.get(path);
    if (s) for (const cb of s.values()) cb(undefined);
  }

  // 渲染器拿到的帮手：事件回灌 + bind 三件套（契约 §2.2 读写契约）
  function helpers(path, props) {
    return {
      fire: (event) => fire(path, event),
      bind: typeof props.bind === 'string' ? {
        path: props.bind,
        get: () => model.get(props.bind),
        subscribe: (cb) => {
          let s = subs.get(props.bind);
          if (!s) subs.set(props.bind, (s = new Map()));
          s.set(path, cb);
        },
        commit: (value) => commitModel(props.bind, value),
      } : null,
    };
  }

  function applyOps(ops) {
    if (!Array.isArray(ops)) throw new Error('ops payload must be an array');
    for (const op of ops) {
      try {
        applyOp(op);
      } catch (e) {
        // JS 侧错误不静默：走 onError（demo 打到 shell；worker 形态
        // 回灌 bgerror 事件，契约 §6）
        onError(`${op.op} ${op.path ?? ''}: ${e.message ?? e}`);
      }
    }
  }

  function applyOp(op) {
    switch (op.op) {
      case 'create': {
        const renderer = renderers.get(op.type);
        if (!renderer) throw new Error(`unknown component type "${op.type}" (JS 侧无渲染器)`);
        if (widgets.has(op.path)) throw new Error(`widget "${op.path}" already exists`);
        const parent = parentPath(op.path);
        let container = root;
        if (parent) {
          const p = widgets.get(parent);
          if (!p) throw new Error(`parent "${parent}" does not exist`);
          container = p.renderer.container ? p.renderer.container(p.el) : p.el;
        }
        const props = op.props ?? {};
        const el = renderer.create(op.path, props, helpers(op.path, props));
        el.dataset.uiPath = op.path;
        container.appendChild(el);
        widgets.set(op.path, { el, type: op.type, renderer, props });
        break;
      }
      case 'set': {
        const w = widgets.get(op.path);
        if (!w) throw new Error(`unknown widget "${op.path}"`);
        Object.assign(w.props, op.props ?? {});
        if (w.renderer.update) {
          w.renderer.update(w.el, op.props ?? {}, helpers(op.path, w.props));
        }
        break;
      }
      case 'delete': {
        if (!widgets.has(op.path)) throw new Error(`unknown widget "${op.path}"`);
        // 子孙一并删除；先子后父
        const doomed = [...widgets.keys()]
          .filter((p) => p === op.path || p.startsWith(op.path + '.'))
          .sort((a, b) => b.length - a.length);
        for (const p of doomed) {
          const w = widgets.get(p);
          w.renderer.dispose?.(w.el);
          for (const s of subs.values()) s.delete(p);
          w.el.remove();
          widgets.delete(p);
        }
        break;
      }
      case 'model': {
        if (op.sub === 'set') modelSet(op.path, op.value);
        else if (op.sub === 'unset') modelUnset(op.path);
        else throw new Error(`unknown model sub "${op.sub}"`);
        break;
      }
      case 'query':
        throw new Error('query op not implemented (M4+)');
      default:
        throw new Error(`unknown op "${op.op}"`);
    }
  }

  return {
    applyOps,
    registerRenderer,
    modelGet: (path) => model.get(path),
    hasWidget: (path) => widgets.has(path),
  };
}

// ── L0 纯 DOM 渲染器（catalog 见 rtcl-core/src/ui.tcl）──────────────

function h(tag, cls) {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  return el;
}

function applyTextProps(el, props) {
  if ('variant' in props) el.dataset.variant = props.variant;
  if ('text' in props) el.textContent = props.text;
}

export function registerDomRenderers(runtime) {
  runtime.registerRenderer('Surface', {
    create(path, props) {
      const el = h('section', 'rui-surface');
      const title = h('div', 'rui-surface-title');
      title.textContent = props.title ?? '';
      const body = h('div', 'rui-surface-body');
      el.append(title, body);
      return el;
    },
    container: (el) => el.querySelector('.rui-surface-body'),
    update(el, props) {
      if ('title' in props) {
        el.querySelector('.rui-surface-title').textContent = props.title;
      }
    },
  });

  runtime.registerRenderer('Row', { create: () => h('div', 'rui-row') });
  runtime.registerRenderer('Column', { create: () => h('div', 'rui-col') });
  runtime.registerRenderer('Card', { create: () => h('div', 'rui-card') });
  runtime.registerRenderer('Divider', { create: () => h('hr', 'rui-divider') });

  runtime.registerRenderer('Text', {
    create(path, props) {
      const el = h('div', 'rui-text');
      applyTextProps(el, props);
      return el;
    },
    update: applyTextProps,
  });

  runtime.registerRenderer('Button', {
    create(path, props, { fire }) {
      const el = h('button', 'rui-btn');
      el.type = 'button';
      applyTextProps(el, props);
      el.addEventListener('click', () => fire('click'));
      return el;
    },
    update: applyTextProps,
  });

  runtime.registerRenderer('Field', {
    create(path, props, { bind }) {
      const el = h('label', 'rui-field');
      const lab = h('span', 'rui-field-label');
      lab.textContent = props.label ?? '';
      const input = h('input', 'rui-field-input');
      el.append(lab, input);
      if (bind) {
        const v = bind.get();
        if (v !== undefined) input.value = String(v);
        bind.subscribe((v) => { input.value = v === undefined ? '' : String(v); });
        // 契约 §2.2：击键只写本地，提交（Enter/blur）才回写模型
        const commit = () => bind.commit(input.value);
        input.addEventListener('change', commit);
        input.addEventListener('keydown', (e) => { if (e.key === 'Enter') commit(); });
      }
      return el;
    },
    update(el, props) {
      if ('label' in props) el.querySelector('.rui-field-label').textContent = props.label;
    },
  });

  runtime.registerRenderer('Checkbox', {
    create(path, props, { bind }) {
      const el = h('label', 'rui-checkbox');
      const input = h('input', 'rui-checkbox-input');
      input.type = 'checkbox';
      const lab = h('span', 'rui-checkbox-label');
      lab.textContent = props.label ?? '';
      el.append(input, lab);
      if (bind) {
        input.checked = !!bind.get();
        bind.subscribe((v) => { input.checked = !!v; });
        input.addEventListener('change', () => bind.commit(input.checked));
      }
      return el;
    },
    update(el, props) {
      if ('label' in props) el.querySelector('.rui-checkbox-label').textContent = props.label;
    },
  });
}

// ── 默认暗色主题（样式全在这里，渲染器不带样式）────────────────────

export function injectUiStyles() {
  const style = document.createElement('style');
  style.textContent = `
.rui-surface { background:#161b22; border:1px solid #30363d; border-radius:8px; margin:12px; overflow:hidden; }
.rui-surface-title { padding:8px 12px; border-bottom:1px solid #30363d; color:#8b949e; font-size:12px; }
.rui-surface-title:empty { display:none; }
.rui-surface-body { padding:12px; display:flex; flex-direction:column; gap:10px; }
.rui-row { display:flex; flex-direction:row; gap:10px; align-items:center; }
.rui-col { display:flex; flex-direction:column; gap:10px; }
.rui-card { background:#0d1117; border:1px solid #30363d; border-radius:6px; padding:10px; display:flex; flex-direction:column; gap:8px; }
.rui-divider { border:none; border-top:1px solid #30363d; width:100%; margin:0; }
.rui-text { color:#c9d1d9; font-size:13px; white-space:pre-wrap; }
.rui-text[data-variant=h1] { font-size:20px; font-weight:600; }
.rui-text[data-variant=h2] { font-size:17px; font-weight:600; }
.rui-text[data-variant=h3] { font-size:15px; font-weight:600; }
.rui-text[data-variant=caption] { font-size:11px; color:#8b949e; }
.rui-btn { background:#21262d; color:#c9d1d9; border:1px solid #30363d; border-radius:6px; padding:5px 14px; font:inherit; font-size:13px; cursor:pointer; align-self:flex-start; }
.rui-btn:hover { background:#30363d; }
.rui-btn[data-variant=primary] { background:#1f6feb; border-color:#1f6feb; color:#fff; }
.rui-btn[data-variant=primary]:hover { background:#388bfd; }
.rui-btn[data-variant=borderless] { background:transparent; border-color:transparent; color:#58a6ff; padding:5px 6px; }
.rui-btn[data-variant=borderless]:hover { background:#21262d; }
.rui-field { display:flex; flex-direction:column; gap:4px; font-size:12px; color:#8b949e; }
.rui-field-input { background:#0d1117; border:1px solid #30363d; border-radius:6px; color:#c9d1d9; padding:5px 8px; font:inherit; font-size:13px; }
.rui-field-input:focus { outline:none; border-color:#1f6feb; }
.rui-checkbox { display:flex; gap:8px; align-items:center; font-size:13px; color:#c9d1d9; cursor:pointer; }
.rui-checkbox-input { accent-color:#1f6feb; }
`;
  document.head.appendChild(style);
}
