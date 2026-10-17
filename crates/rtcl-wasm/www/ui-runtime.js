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

  // ── 布局：props 里的公共布局键 → el.style（docs/UI-LAYOUT.md §3）──
  // 组件渲染器不感知布局；fill/weight 的主轴语义取决于父容器方向。

  function axisOf(type) {
    if (type === 'Row') return 'x';
    if (type === 'Column' || type === 'Surface' || type === 'Card') return 'y';
    return null;   // 叶子或未知父：按 block 语义退化
  }

  function cssSize(v) {
    const s = String(v);
    return /^[0-9]+(\.[0-9]+)?$/.test(s) ? s + 'px' : s;
  }

  const ALIGN_MAP = { start: 'flex-start', end: 'flex-end', center: 'center', stretch: 'stretch' };

  function applyLayout(el, props, axis) {
    if ('gap' in props) el.style.gap = cssSize(props.gap);
    if ('pad' in props) el.style.padding = cssSize(props.pad);
    if ('width' in props) el.style.width = cssSize(props.width);
    if ('height' in props) el.style.height = cssSize(props.height);
    if ('align' in props) el.style.alignSelf = ALIGN_MAP[props.align] ?? props.align;
    if ('weight' in props) el.style.flex = `${props.weight} 1 0`;
    if ('fill' in props) {
      const f = props.fill;
      if (axis === null) {
        if (f === 'x' || f === 'both') el.style.width = '100%';
        if (f === 'y' || f === 'both') el.style.height = '100%';
      } else {
        const crossStretch = axis === 'x' ? (f === 'y' || f === 'both') : (f === 'x' || f === 'both');
        const mainFill = axis === 'x' ? (f === 'x' || f === 'both') : (f === 'y' || f === 'both');
        if (crossStretch) el.style.alignSelf = 'stretch';
        // flex 无"槽"概念：主轴填满等价于争抢剩余（显式 -weight 优先）
        if (mainFill && !('weight' in props)) el.style.flexGrow = '1';
      }
    }
  }

  function modelSet(path, value) {
    model.set(path, value);
    notifySubscribers(path, value);
  }

  function modelUnset(path) {
    model.delete(path);
    notifySubscribers(path, undefined);
  }

  // 订阅扇出（契约 §2.2 的增量语义）：写 /pods/42 时，除了 /pods/42
  // 自己的订阅者，**祖先路径**的订阅者也收到通知——ui::table 只订
  // 阅 /pods，靠它拿到行级增量而不全量重渲染。回调第二参 = 实际变更
  // 的路径（精确订阅者拿到的是原样 value；祖先订阅者拿到的是变更
  // 子路径上的值，自己决定如何合并——Table 按 /pods/<i> 整行）。
  // Field/Checkbox 的回调只用第一参，签名不变。
  function notifySubscribers(changedPath, value) {
    for (const [p, s] of subs) {
      if (p === changedPath || changedPath.startsWith(p + '/')) {
        for (const cb of s.values()) cb(value, changedPath);
      }
    }
  }

  // 渲染器拿到的帮手：事件回灌 + bind 三件套 + events（契约 §2.2 读写
  // 契约）。events 是 create/set op 里的转发事件名数组——-command
  // 一类脚本留在 Tcl 侧，JS 只凭名字画交互元素（Table 的行动作按钮
  // 就是 events 里的 row<K>）。fire 的可选第二参是行级 detail
  // （Table 行号），宿主把它拼进回灌命令 "…::ui::fire <path> <ev> <detail>"。
  function helpers(path, props, events) {
    return {
      fire: (event, detail) => fire(path, event, detail),
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
      events: events ?? [],
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
        let parentType = null;
        if (parent) {
          const p = widgets.get(parent);
          if (!p) throw new Error(`parent "${parent}" does not exist`);
          container = p.renderer.container ? p.renderer.container(p.el) : p.el;
          parentType = p.type;
        }
        const props = op.props ?? {};
        const el = renderer.create(op.path, props, helpers(op.path, props, op.events));
        el.dataset.uiPath = op.path;
        applyLayout(el, props, axisOf(parentType));
        container.appendChild(el);
        widgets.set(op.path, { el, type: op.type, renderer, props, parentType });
        break;
      }
      case 'set': {
        const w = widgets.get(op.path);
        if (!w) throw new Error(`unknown widget "${op.path}"`);
        Object.assign(w.props, op.props ?? {});
        applyLayout(w.el, op.props ?? {}, axisOf(w.parentType));
        if (w.renderer.update) {
          w.renderer.update(w.el, op.props ?? {}, helpers(op.path, w.props, op.events));
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
// ── Table：行数组 + 虚拟滚动（docs/UI-RESEARCH.md §2.1/§5）────────
// 反模式自查：大数据绝不一次性全量渲染 DOM。固定行高 + 绝对定位
// spacer 只维护可视区 ± 缓冲的行节点；模型子路径增量（ui::model set
// /pods/42 只携带一行）原位更新那一行，其他行节点不动（不重排）。
// 滚动与行事件都是 View 本地先行（契约 §2.2/§4），不触 Tcl。
// 几何常量是可视区计算所需的结构参数（JS 必须知道行高），不是样式；
// 视觉全部在 injectUiStyles，改样式须与 TABLE_ROW_H 同步（含边框）。
const TABLE_ROW_H = 28;
const TABLE_BUF = 8;   // 可视区上下各缓冲的行数

function tableScheduleWindow(st) {
  if (st.pending) return;
  st.pending = true;
  requestAnimationFrame(() => { st.pending = false; tableRenderWindow(st); });
}

// 窗口 [first, last) 内的行节点仓管：新增缺失、移除越界可见（内容
// 由 fillRow 写入只在数据变化时发生，滚动只动 Membership）。
function tableRenderWindow(st) {
  const height = st.el.clientHeight || 320;
  const first = Math.max(0, Math.floor(st.el.scrollTop / TABLE_ROW_H) - TABLE_BUF);
  const end = Math.min(st.n, first + Math.ceil(height / TABLE_ROW_H) + TABLE_BUF * 2);
  for (const [idx, rel] of st.pool) {
    if (idx < first || idx >= end) { rel.remove(); st.pool.delete(idx); }
  }
  for (let i = first; i < end; i++) {
    let r = st.pool.get(i);
    if (r === undefined) {
      r = tableMakeRow(st, i);
      st.spacer.appendChild(r);
      st.pool.set(i, r);
      tableFillRow(st, r);
    }
  }
}

function tableMakeRow(st, idx) {
  const r = h('div', 'rui-row');
  r.dataset.idx = idx;
  r.style.transform = `translateY(${idx * TABLE_ROW_H}px)`;
  r._cells = [];
  for (const c of st.cols) {
    const d = h('div', 'rui-td');
    d.dataset.col = c;
    r._cells.push(d);
    r.appendChild(d);
  }
  if (st.actions.length) {
    const zone = h('div', 'rui-td-acts');
    for (const a of st.actions) {
      const b = h('button', 'rui-act');
      b.type = 'button';
      b.textContent = a;
      b.dataset.act = a;
      zone.appendChild(b);
    }
    r.appendChild(zone);
  }
  return r;
}

function tableFillRow(st, r) {
  const row = st.rows[+r.dataset.idx];
  for (let ci = 0; ci < st.cols.length; ci++) {
    const v = row == null ? undefined : row[st.cols[ci]];
    // 只写 textContent：值来自 JSON，转义即安全（不拼 HTML）
    r._cells[ci].textContent = v == null ? ''
      : typeof v === 'object' ? JSON.stringify(v) : String(v);
  }
}

function tableSetDataAll(st, v) {
  if (v === undefined) st.rows = [];
  else if (Array.isArray(v)) st.rows = v;
  else throw new Error(`Table ${st.path}: model "${st.bindPath}" must be a JSON array of rows`);
  st.n = st.rows.length;
  st.spacer.style.height = (st.n * TABLE_ROW_H) + 'px';
  st.spacer.replaceChildren();   // 全量替换：窗口行节点重建
  st.pool.clear();
  tableRenderWindow(st);
}

function tableRowDelta(st, i, v) {
  st.rows[i] = v;
  if (i + 1 > st.n) {
    st.n = i + 1;
    st.spacer.style.height = (st.n * TABLE_ROW_H) + 'px';
  }
  const r = st.pool.get(i);
  if (r !== undefined) tableFillRow(st, r);   // 可视区内：原位写 cell（不重建）
  // 游标喂数的可视区自亮：行进窗内时由下一帧补渲染（scroll 才渲染
  // 的话，停在 scrollTop=0 的用户看着 10 万行流过仍是空表）。rAF 合
  // 并吞掉批内全量 schedule，每帧至多一次 membership 差分。
  tableScheduleWindow(st);
}

// 订阅回调（前缀扇出，第二参 = 实际变更路径）：
function tableOnModelEvent(st, sub, v) {
  if (sub === undefined || sub === st.bindPath) { tableSetDataAll(st, v); return; }
  const rest = sub.slice(st.bindPath.length + 1);
  if (/^\d+$/.test(rest)) tableRowDelta(st, +rest, v);
  // cell 级子路径（/pods/42/status）v1 不进：行在模型树存原始
  // JSON 串，dict 语义不支持下钻（ui.tcl Table 注释 / 变更建议 §8）。
}

// events 数组（Tcl 侧转发标记）→ 行尾行动作按钮：row<名> 减
// rowclick。变化时整窗行节点重建（结构变化，少见路径）。
function tableResolveActions(st, events) {
  const acts = (Array.isArray(events) ? events : [])
    .filter(e => e !== 'rowclick' && e.startsWith('row'))
    .map(e => e.slice(3));
  const key = acts.join('|');
  if (key !== st.actsKey) {
    st.actsKey = key;
    st.actions = acts;
    tableBuildHead(st);
    st.spacer.replaceChildren();
    st.pool.clear();
    tableRenderWindow(st);
  }
}

// 表头：列标签 + 隐形动作按钮替身（与行尾动作区同宽，列对齐）
function tableBuildHead(st) {
  st.head.replaceChildren();
  for (const c of st.cols) {
    const d = h('div', 'rui-td');
    d.textContent = c;
    st.head.appendChild(d);
  }
  if (st.actions.length) {
    const zone = h('div', 'rui-td-acts');
    for (const a of st.actions) {
      const b = h('button', 'rui-act');
      b.type = 'button';
      b.textContent = a;
      zone.appendChild(b);
    }
    st.head.appendChild(zone);
  }
}

// 事件委托（容器级一次挂接）：行动作按钮 → row<K>，行身 → rowclick。
// detail = 行号，宿主拼成 "::ui::fire <path> <event> <row>" 回灌。
function tableOnRowClick(st, hp, e) {
  const rowEl = e.target.closest('.rui-row');
  if (!rowEl) return;
  const act = e.target.closest('.rui-act');
  hp.fire(act ? 'row' + act.dataset.act : 'rowclick', rowEl.dataset.idx);
}

runtime.registerRenderer('Table', {
  create(path, props, hp) {
    const el = h('div', 'rui-table');
    const head = h('div', 'rui-table-head');
    const spacer = h('div', 'rui-table-spacer');
    el.append(head, spacer);
    const st = { path, bindPath: typeof hp.bind === 'object' ? hp.bind.path : null, el, head, spacer, cols: [], rows: [], n: 0, pool: new Map(), actsKey: '', actions: [], pending: false };
    el._table = st;
    if (Array.isArray(props.columns)) st.cols = props.columns.map(String);
    else if (props.columns !== undefined) throw new Error(`Table ${path}: -columns must be a JSON array of column names`);
    tableBuildHead(st);
    tableResolveActions(st, hp.events);
    if (hp.bind) {
      hp.bind.subscribe((v, sub) => tableOnModelEvent(st, sub, v));
      const init = hp.bind.get();
      if (init !== undefined) tableSetDataAll(st, init);
    }
    el.addEventListener('scroll', () => tableScheduleWindow(st), { passive: true });
    el.addEventListener('click', (e) => tableOnRowClick(st, hp, e));
    tableRenderWindow(st);
    return el;
  },
  update(el, props, hp) {
    const st = el._table;
    if ('columns' in props) {
      if (!Array.isArray(props.columns)) throw new Error(`Table ${st.path}: -columns must be a JSON array of column names`);
      st.cols = props.columns.map(String);
      tableBuildHead(st);
      st.spacer.replaceChildren();
      st.pool.clear();
      tableRenderWindow(st);
    }
    tableResolveActions(st, hp.events);
  },
});

// ── Log：追加式日志流（UI-RESEARCH §3.2）────────────────────────
// 模型是行数组（ui::model set /deploylog/i 一次一行，与 Table 同款
// 游标/前缀扇出）。DOM 只留窗口行：spacer 撑总高 + 行 translateY
// 绝对定位（与 Table 同构）。行值 JSON 对象的 level 字段挂颜色类
// （样式在 injectUiStyles；类名只取词表字符，未知/非法落 info）；
// 标量行值视为 info。follow：贴底时新行自动钉底；用户上滚即暂停，
// 滚回底部自动恢复。行事件 lineclick 容器委托（events 含 lineclick
// 才挂，事件名是 Tcl 侧的转发开关）。正则高亮 v1 不做。
const LOG_ROW_H = 22;
const LOG_BUF = 8;

function logScheduleWindow(st) {
  if (st.pending) return;
  st.pending = true;
  requestAnimationFrame(() => { st.pending = false; logRenderWindow(st); });
}

function logRenderWindow(st) {
  const height = st.el.clientHeight || 320;
  const first = Math.max(0, Math.floor(st.el.scrollTop / LOG_ROW_H) - LOG_BUF);
  const end = Math.min(st.n, first + Math.ceil(height / LOG_ROW_H) + LOG_BUF * 2);
  for (const [idx, rel] of st.pool) {
    if (idx < first || idx >= end) { rel.remove(); st.pool.delete(idx); }
  }
  for (let i = first; i < end; i++) {
    let r = st.pool.get(i);
    if (r === undefined) {
      r = logMakeLine(st, i);
      st.spacer.appendChild(r);
      st.pool.set(i, r);
      logFillLine(st, r);
    }
  }
}

function logMakeLine(st, idx) {
  const r = h('div', 'rui-log-line');
  r.dataset.idx = idx;
  r.style.transform = `translateY(${idx * LOG_ROW_H}px)`;
  r._text = h('span', 'rui-log-text');
  r.appendChild(r._text);
  return r;
}

function logFillLine(st, r) {
  const row = st.rows[+r.dataset.idx];
  let text = '';
  let level = 'info';
  if (row != null) {
    if (typeof row === 'object') {
      text = row.text == null ? '' : String(row.text);
      // level 进 CSS 类名：只放词表字符，其余落默认（类名注入边界）
      level = /^[A-Za-z0-9_-]+$/.test(String(row.level ?? '')) ? String(row.level) : 'info';
    } else {
      text = String(row);
    }
  }
  r.className = `rui-log-line rui-lv-${level}`;
  r._text.textContent = text;   // 值来自 JSON，转义即安全（不拼 HTML）
}

function logSetDataAll(st, v) {
  if (v === undefined) st.rows = [];
  else if (Array.isArray(v)) st.rows = v;
  else throw new Error(`Log ${st.path}: model "${st.bindPath}" must be a JSON array of lines`);
  st.n = st.rows.length;
  st.spacer.style.height = (st.n * LOG_ROW_H) + 'px';
  st.spacer.replaceChildren();   // 全量替换：窗口行节点重建
  st.pool.clear();
  if (st.following) st.el.scrollTop = st.el.scrollHeight;   // 全量后仍钉底
  logRenderWindow(st);
}

function logLineDelta(st, i, v) {
  st.rows[i] = v;
  if (i + 1 > st.n) {
    st.n = i + 1;
    st.spacer.style.height = (st.n * LOG_ROW_H) + 'px';
  }
  const r = st.pool.get(i);
  if (r !== undefined) logFillLine(st, r);   // 窗口内：原位写行
  if (st.following) st.el.scrollTop = st.el.scrollHeight;   // follow 钉底
  logScheduleWindow(st);   // 游标追加的窗口自亮（同 Table 的增量路径）
}

function logOnModelEvent(st, sub, v) {
  if (sub === undefined || sub === st.bindPath) { logSetDataAll(st, v); return; }
  const rest = sub.slice(st.bindPath.length + 1);
  if (/^\d+$/.test(rest)) logLineDelta(st, +rest, v);
}

runtime.registerRenderer('Log', {
  create(path, props, hp) {
    const el = h('div', 'rui-log');
    const spacer = h('div', 'rui-log-spacer');
    el.append(spacer);
    const st = { path, bindPath: typeof hp.bind === 'object' ? hp.bind.path : null, el, spacer, rows: [], n: 0, pool: new Map(), pending: false, following: true };
    el._log = st;
    if (hp.bind) {
      hp.bind.subscribe((v, sub) => logOnModelEvent(st, sub, v));
      const init = hp.bind.get();
      if (init !== undefined) logSetDataAll(st, init);
    }
    el.addEventListener('scroll', () => {
      st.following = st.el.scrollTop + st.el.clientHeight >= st.el.scrollHeight - LOG_ROW_H;
      logScheduleWindow(st);
    }, { passive: true });
    if (Array.isArray(hp.events) && hp.events.includes('lineclick')) {
      el.addEventListener('click', (e) => {
        const line = e.target.closest('.rui-log-line');
        if (line) hp.fire('lineclick', line.dataset.idx);
      });
    }
    logRenderWindow(st);
    return el;
  },
  update(el, props, hp) { /* v1: Log 无专有可更新选项（-path 变化走重建） */ },
});
}

// ── 默认暗色主题（样式全在这里，渲染器不带样式）────────────────────────

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
/* ui::table —— 虚拟滚动容器；行高 28px 须与 ui-runtime.js 的
   TABLE_ROW_H 同步（border 计入高度，box-sizing:border-box 锁定）。 */
.rui-table { position:relative; height:320px; overflow-y:auto; background:#0d1117; border:1px solid #30363d; border-radius:6px; font-size:12px; color:#c9d1d9; }
.rui-table-head { position:sticky; top:0; z-index:2; display:flex; align-items:center; height:28px; box-sizing:border-box; background:#161b22; color:#8b949e; font-weight:600; border-bottom:1px solid #30363d; }
.rui-table-head .rui-act { visibility:hidden; pointer-events:none; }
.rui-table-spacer { position:relative; }
.rui-row { position:absolute; left:0; right:0; top:0; height:28px; box-sizing:border-box; display:flex; align-items:center; border-bottom:1px solid #21262d; cursor:pointer; }
.rui-row:hover { background:#161b22; }
.rui-td { flex:1 1 0; min-width:0; padding:0 10px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.rui-td-acts { flex:0 0 auto; display:flex; gap:6px; padding:0 8px; }
.rui-act { background:#21262d; color:#58a6ff; border:1px solid #30363d; border-radius:4px; padding:2px 8px; font:inherit; font-size:11px; cursor:pointer; }
.rui-act:hover { background:#30363d; }
/* ui::log —— 追加式日志流；行高 22px 须与 ui-runtime.js 的
   LOG_ROW_H 同步（box-sizing:border-box 锁定）。 */
.rui-log { position:relative; height:320px; overflow-y:auto; background:#0a0c10; border:1px solid #30363d; border-radius:6px; font:12px/22px ui-monospace, SFMono-Regular, Menlo, monospace; color:#c9d1d9; }
.rui-log-spacer { position:relative; }
.rui-log-line { position:absolute; left:0; right:0; top:0; height:22px; box-sizing:border-box; display:flex; align-items:center; white-space:nowrap; overflow:hidden; }
.rui-log-line.rui-lv-warn { color:#d29922; }
.rui-log-line.rui-lv-error { color:#f85149; }
.rui-log-line.rui-lv-debug { color:#8b949e; }
`;
  document.head.appendChild(style);
}
