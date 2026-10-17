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
    // Tk -weight 语义：分配（父容器的）额外空间，保留自然尺寸。
    // flex-basis 必须是 auto——basis 0 会把固定设计高的组件在
    // auto 高的 flex column 父里塌到零贡献（rect 2px，M5 的 table
    // 就因此整体不可见，被 clientHeight||320 的虚拟化回退掩盖）。
    if ('weight' in props) el.style.flex = `${props.weight} 1 auto`;
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
      // 通用模型访问（-vegalite 一类：数据路径写在 spec 里，而不是
      // props.bind）。watch 与前缀扇出同语义，返回退订函数。
      read: (modelPath) => model.get(modelPath),
      watch: (modelPath, cb) => {
        let s = subs.get(modelPath);
        if (!s) subs.set(modelPath, (s = new Map()));
        s.set(path, cb);
        return () => { const cur = subs.get(modelPath); if (cur) cur.delete(path); };
      },
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

// 行事件绑定：create 与 update（ui::on 后挂 → set op 携带 events）都
// 进这里；幂等（只挂一次）。事件名是 Tcl 侧的转发开关。
function logBindEvents(st, el, events) {
  if (st.clickBound || !Array.isArray(events) || !events.includes('lineclick')) return;
  st.clickBound = true;
  el.addEventListener('click', (e) => {
    const line = e.target.closest('.rui-log-line');
    if (line) st.fire('lineclick', line.dataset.idx);
  });
}

runtime.registerRenderer('Log', {
  create(path, props, hp) {
    const el = h('div', 'rui-log');
    const spacer = h('div', 'rui-log-spacer');
    el.append(spacer);
    const st = { path, bindPath: typeof hp.bind === 'object' ? hp.bind.path : null, el, spacer, rows: [], n: 0, pool: new Map(), pending: false, following: true, clickBound: false };
    el._log = st;
    if (hp.bind) {
      hp.bind.subscribe((v, sub) => logOnModelEvent(st, sub, v));
      const init = hp.bind.get();
      if (init !== undefined) logSetDataAll(st, init);
    }
    st.fire = hp.fire;
    logBindEvents(st, el, hp.events);
    el.addEventListener('scroll', () => {
      st.following = st.el.scrollTop + st.el.clientHeight >= st.el.scrollHeight - LOG_ROW_H;
      logScheduleWindow(st);
    }, { passive: true });
    logRenderWindow(st);
    return el;
  },
  update(el, props, hp) {
    // v1: Log 无专有可更新选项（-path 变化走重建）；事件走这里后挂
    const st = el._log;
    if (!st) return;
    st.fire = hp.fire;
    logBindEvents(st, el, hp.events);
  },
});

// ── Kv：键值详情（L1 详情双件套之一；UI-RESEARCH §2.3/§3）────────
// 绑定一个模型对象，渲染 键|值 两列（标量原样；嵌套对象/数组
// compact JSON 单行——下钻用 ui::tree）。键序：-keys 显式优先（只
// 渲染列出的键），否则模型插入序。子路径增量原位更新单行；嵌套
// 子路径（/info/a/b）合成进本地视图后重绘（详情对象小，重建成本
// 可忽略）。
//
// 运行时 model 是 path->值 的平表（写 /info/a 不改 /info 对象本身，
// Table 的 /pods/i 同源设计）：容器组件按增量把子路径写进自己的
// 本地视图，全量 set 根路径时以根值为权威快照。
function viewDelta(root, segs, value) {
  if (segs.length === 0) return value;
  const obj = (root !== null && typeof root === 'object') ? root : {};
  let cur = obj;
  for (let i = 0; i < segs.length - 1; i++) {
    const k = segs[i];
    const nxt = cur[k];
    if (nxt === null || typeof nxt !== 'object') cur[k] = {};
    cur = cur[k];
  }
  const last = segs[segs.length - 1];
  if (value === undefined) delete cur[last]; else cur[last] = value;
  return obj;
}

function kvFmt(v) {
  if (v === null) return 'null';
  if (typeof v === 'object') return JSON.stringify(v);
  return String(v);
}

function kvRender(st) {
  const v = st.value;
  if (v !== undefined && v !== null && typeof v !== 'object') {
    throw new Error(`Kv ${st.path}: model "${st.bindPath}" must be a JSON object`);
  }
  let keys = [];
  if (v !== undefined && v !== null) {
    const own = Object.keys(v);
    const wanted = Array.isArray(st.keysOrder) && st.keysOrder.length ? st.keysOrder : own;
    keys = wanted.filter(k => own.includes(k));   // -keys 列出的缺失键跳过
  }
  st.rows.clear();
  if (v === undefined || keys.length === 0) {
    const empty = h('div', 'rui-kv-empty');
    empty.textContent = st.bindPath ? '(no entries)' : '(no -path)';
    st.el.replaceChildren(empty);
    return;
  }
  const nodes = [];
  for (const k of keys) {
    const row = h('div', 'rui-kv-row');
    const kEl = h('div', 'rui-kv-k');
    kEl.textContent = k;
    kEl.title = k;
    const vEl = h('div', 'rui-kv-v');
    vEl.textContent = kvFmt(v[k]);
    row.append(kEl, vEl);
    nodes.push(row);
    st.rows.set(k, { row, vEl });
  }
  st.el.replaceChildren(...nodes);
}

function kvOnModelEvent(st, sub, v) {
  if (sub === undefined || sub === st.bindPath) { st.value = v; kvRender(st); return; }
  const segs = sub.slice(st.bindPath.length + 1).split('/');
  st.value = viewDelta(st.value, segs, v);
  const row = segs.length === 1 ? st.rows.get(segs[0]) : undefined;
  if (row !== undefined && v !== undefined) {
    row.vEl.textContent = kvFmt(v);   // 单键原位更新（不重建其它行）
  } else {
    kvRender(st);                     // 嵌套/新键/删除：重建本地视图
  }
}

runtime.registerRenderer('Kv', {
  create(path, props, hp) {
    const el = h('div', 'rui-kv');
    const st = {
      path, bindPath: typeof hp.bind === 'object' ? hp.bind.path : null,
      el, value: undefined, keysOrder: Array.isArray(props.keys) ? props.keys : [],
      rows: new Map(),
    };
    el._kv = st;
    if (hp.bind) {
      hp.bind.subscribe((v, sub) => kvOnModelEvent(st, sub, v));
      const init = hp.bind.get();
      if (init !== undefined) st.value = init;
    }
    kvRender(st);
    return el;
  },
  update(el, props, hp) {
    const st = el._kv;
    if (!st) return;
    if ('keys' in props) { st.keysOrder = Array.isArray(props.keys) ? props.keys : []; kvRender(st); }
  },
});

// ── Tree：JSON 树（详情双件套之二）──────────────────────────────
// 绑定任意 JSON 值。容器（对象/数组）默认折叠并给预览计数；展开
// 状态存在客户端（Set，id = 段路径的 JSON 编码，键里带 "/" 也不
// 歧义），每次切换/模型更新按可见节点重建（DOM 只物化展开路径——
// "懒加载子节点"）。标量节点单行。u::on 类行事件 v1 不做。
const TREE_INDENT = 14;

function treeIsContainer(v) { return v !== null && typeof v === 'object'; }
function treePreview(v) {
  if (Array.isArray(v)) return `[…] ${v.length} items`;
  return `{…} ${Object.keys(v).length} keys`;
}
function treeScalar(v) {
  if (v === undefined) return '(deleted)';
  if (v === null) return 'null';
  if (typeof v === 'object') return Array.isArray(v) ? `[… ${v.length}]` : `{… ${Object.keys(v).length}}`;
  return String(v);
}

function treeRender(st) {
  const lines = [];
  const walk = (value, label, depth, segs) => {
    const id = JSON.stringify(segs);
    const row = h('div', 'rui-tree-row');
    row.dataset.id = id;
    row.style.paddingLeft = (depth * TREE_INDENT) + 'px';
    const arrow = h('span', 'rui-tree-arrow');
    const lab = h('span', 'rui-tree-label');
    lab.textContent = label;
    row.append(arrow, lab);
    if (treeIsContainer(value)) {
      const open = st.expanded.has(id);
      arrow.textContent = open ? '▾' : '▸';
      row.classList.add('rui-tree-cont');
      if (!open) {
        const prev = h('span', 'rui-tree-prev');
        prev.textContent = treePreview(value);
        row.append(prev);
        lines.push(row);
        return;
      }
      lines.push(row);
      const childKeys = Array.isArray(value) ? value.map((_, i) => i) : Object.keys(value);
      for (const ck of childKeys) {
        const childLabel = Array.isArray(value) ? `[${ck}]` : String(ck);
        walk(value[ck], childLabel, depth + 1, segs.concat([typeof ck === 'number' ? '#' + ck : 'k:' + ck]));
      }
    } else {
      arrow.textContent = ' ';
      const val = h('span', 'rui-tree-val');
      val.textContent = treeScalar(value);
      row.append(val);
      lines.push(row);
    }
  };
  const root = st.value;
  const rootLabel = treeIsContainer(root) ? '(root)' : 'value';
  if (root === undefined) {
    st.el.replaceChildren();
    return;
  }
  walk(root, rootLabel, 0, []);
  st.el.replaceChildren(...lines);
}

runtime.registerRenderer('Tree', {
  create(path, props, hp) {
    const el = h('div', 'rui-tree');
    const st = {
      path, bindPath: typeof hp.bind === 'object' ? hp.bind.path : null,
      el, value: undefined, expanded: new Set(),
    };
    el._tree = st;
    if (hp.bind) {
      hp.bind.subscribe((v, sub) => {
        if (sub === undefined || sub === st.bindPath) st.value = v;
        else st.value = viewDelta(st.value, sub.slice(st.bindPath.length + 1).split('/'), v);
        treeRender(st);
      });
      const init = hp.bind.get();
      if (init !== undefined) st.value = init;
    }
    el.addEventListener('click', (e) => {
      const row = e.target.closest('.rui-tree-row');
      if (!row || !row.classList.contains('rui-tree-cont')) return;
      const id = row.dataset.id;
      if (st.expanded.has(id)) st.expanded.delete(id); else st.expanded.add(id);
      treeRender(st);
    });
    treeRender(st);
    return el;
  },
  update(el, props, hp) {
    // v1: -path 变化走重建；布局选项由 runtime 统一应用
  },
});

// ── Chart：Vega-Lite 子集（L1 数据分析组件；UI-RESEARCH §2.2）──
// 项目零新依赖：不引 vega-lite 库，内置常见声明式子集——mark
// line/bar/point/area + x/y 的 field 编码；x 按序数等距摆放（时序
// 标签原样显示）。数据走 model 通道：spec.data.name 是模型路径
// （行数组，游标喂数），渲染器订阅该前缀，增量 append 合并到一帧
// 重绘（spec 不变，组件不重建）。spec 结构错误在 create/update
// 同步抛（applyOp 的 try/catch → onError）；渲染阶段只做防御性
// 数据跳过（非对象行 / 非数值 y），不抛错。
const CHART_MARKS = ['line', 'bar', 'point', 'area'];

function chartFmt(v) {
  const a = Math.abs(v);
  if (a >= 1000) return String(Math.round(v));
  return String(Math.round(v * 100) / 100);
}

function chartValidate(path, spec) {
  if (spec === null || typeof spec !== 'object' || Array.isArray(spec)) {
    throw new Error(`Chart ${path}: -vegalite must be a JSON object`);
  }
  const enc = spec.encoding;
  if (!enc || typeof enc !== 'object' || !enc.x || !enc.y
      || typeof enc.x.field !== 'string' || typeof enc.y.field !== 'string') {
    throw new Error(`Chart ${path}: spec.encoding.x.field and spec.encoding.y.field are required`);
  }
  const mark = typeof spec.mark === 'string' ? spec.mark : (spec.mark && spec.mark.type);
  if (!CHART_MARKS.includes(mark)) {
    throw new Error(`Chart ${path}: unsupported mark "${mark}" (v1: ${CHART_MARKS.join('/')})`);
  }
  if (spec.data !== undefined) {
    const name = spec.data && spec.data.name;
    if (typeof name !== 'string' || name === '') {
      throw new Error(`Chart ${path}: spec.data.name must be a model path`);
    }
  }
}

function chartSchedule(st) {
  if (st.pending) return;
  st.pending = true;
  requestAnimationFrame(() => { st.pending = false; chartRender(st); });
}

function chartRender(st) {
  const spec = st.spec;
  st.plot.replaceChildren();
  if (!spec) return;
  const xf = spec.encoding.x.field;
  const yf = spec.encoding.y.field;
  const mark = typeof spec.mark === 'string' ? spec.mark : spec.mark.type;
  const pts = [];
  for (let i = 0; i < st.n; i++) {
    const r = st.rows[i];
    if (r === null || typeof r !== 'object' || Array.isArray(r)) continue;
    const y = Number(r[yf]);
    if (!Number.isFinite(y)) continue;
    pts.push({ x: r[xf] === undefined || r[xf] === null ? '' : String(r[xf]), y });
  }
  if (pts.length === 0) {
    const empty = h('div', 'rui-chart-empty');
    empty.textContent = '(no data)';
    st.plot.append(empty);
    return;
  }
  let lo = Infinity;
  let hi = -Infinity;
  for (const p of pts) { if (p.y < lo) lo = p.y; if (p.y > hi) hi = p.y; }
  if (lo === hi) { lo -= 1; hi += 1; }
  const pad = (hi - lo) * 0.08;
  lo -= pad;
  hi += pad;
  const yPct = (v) => (1 - (v - lo) / (hi - lo)) * 100;
  const n = pts.length;
  const xPct = (i) => (n === 1 ? 50 : (i / (n - 1)) * 100);

  // y 网格 + 刻度（5 档；v1 线性均分，不取整"nice"数）
  for (let k = 0; k <= 4; k++) {
    const v = lo + (hi - lo) * k / 4;
    const grid = h('div', 'rui-chart-grid');
    grid.style.top = yPct(v) + '%';
    st.plot.append(grid);
    const lab = h('div', 'rui-chart-ytick');
    lab.style.top = yPct(v) + '%';
    lab.textContent = chartFmt(v);
    st.plot.append(lab);
  }
  // x 标签（最多 7 个，等步采样）
  const step = Math.max(1, Math.ceil(n / 7));
  for (let i = 0; i < n; i += step) {
    const lab = h('div', 'rui-chart-xtick');
    lab.style.left = xPct(i) + '%';
    lab.textContent = pts[i].x;
    st.plot.append(lab);
  }
  if (mark === 'bar') {
    const band = 100 / n;
    for (let i = 0; i < n; i++) {
      const bar = h('div', 'rui-chart-bar');
      bar.style.left = (i * band + band * 0.12) + '%';
      bar.style.width = (band * 0.76) + '%';
      bar.style.height = (100 - yPct(pts[i].y)) + '%';
      st.plot.append(bar);
    }
    return;
  }
  if (mark === 'point') {
    for (let i = 0; i < n; i++) {
      const dot = h('div', 'rui-chart-dot');
      dot.style.left = xPct(i) + '%';
      dot.style.top = yPct(pts[i].y) + '%';
      st.plot.append(dot);
    }
    return;
  }
  // line / area：SVG 用 100×100 百分比坐标（preserveAspectRatio=none +
  // vector-effect 保描边宽度）——无需像素测量，尺寸变化不重算。
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('class', 'rui-chart-svg');
  svg.setAttribute('viewBox', '0 0 100 100');
  svg.setAttribute('preserveAspectRatio', 'none');
  const coords = pts.map((p, i) => `${xPct(i)},${yPct(p.y)}`);
  if (mark === 'area') {
    const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
    poly.setAttribute('points', `0,100 ${coords.join(' ')} 100,100`);
    poly.setAttribute('class', 'rui-chart-area');
    svg.append(poly);
  }
  const line = document.createElementNS('http://www.w3.org/2000/svg', 'polyline');
  line.setAttribute('points', coords.join(' '));
  line.setAttribute('class', 'rui-chart-line');
  line.setAttribute('vector-effect', 'non-scaling-stroke');
  svg.append(line);
  st.plot.append(svg);
}

function chartOnModelEvent(st, v, sub) {
  if (sub === st.dataPath) {
    st.rows = Array.isArray(v) ? v : [];
    st.n = st.rows.length;
  } else {
    const rest = sub.slice(st.dataPath.length + 1);
    if (/^\d+$/.test(rest)) {
      st.rows[+rest] = v;
      if (+rest + 1 > st.n) st.n = +rest + 1;
    }
  }
  chartSchedule(st);
}

function chartInit(st, props, hp) {
  const spec = props.vegalite;
  if (spec !== undefined && spec !== null) chartValidate(st.path, spec);   // 结构错误 → 同步抛给 applyOp
  if (st.unwatch) { st.unwatch(); st.unwatch = null; }
  st.spec = null;
  st.dataPath = null;
  st.rows = [];
  st.n = 0;
  if (spec !== undefined && spec !== null) {
    st.spec = spec;
    const name = spec.data && typeof spec.data.name === 'string' ? spec.data.name : null;
    if (name) {
      st.dataPath = name;
      const init = hp.read(name);
      if (Array.isArray(init)) { st.rows = init; st.n = init.length; }
      st.unwatch = hp.watch(name, (v, sub) => chartOnModelEvent(st, v, sub));
    }
  }
  chartSchedule(st);
}

runtime.registerRenderer('Chart', {
  create(path, props, hp) {
    const el = h('div', 'rui-chart');
    const plot = h('div', 'rui-chart-plot');
    el.append(plot);
    const st = { path, el, plot, spec: null, dataPath: null, rows: [], n: 0, pending: false, unwatch: null };
    el._chart = st;
    chartInit(st, props, hp);
    return el;
  },
  update(el, props, hp) {
    const st = el._chart;
    if (!st) return;
    if ('vegalite' in props) chartInit(st, props, hp);
  },
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
/* ui::kv —— 键值详情；行不虚拟化（详情对象小），长值换行。 */
.rui-kv { background:#0d1117; border:1px solid #30363d; border-radius:6px; font-size:12px; color:#c9d1d9; overflow:hidden; }
.rui-kv-row { display:flex; gap:12px; padding:4px 10px; border-bottom:1px solid #21262d; align-items:flex-start; }
.rui-kv-row:last-child { border-bottom:none; }
.rui-kv-k { flex:0 0 28%; max-width:28%; color:#8b949e; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.rui-kv-v { flex:1 1 auto; min-width:0; white-space:pre-wrap; word-break:break-word; }
.rui-kv-empty { padding:8px 10px; color:#8b949e; font-size:12px; }
/* ui::tree —— JSON 树；容器行可点（▸/▾），展开才物化子节点。 */
.rui-tree { background:#0d1117; border:1px solid #30363d; border-radius:6px; max-height:320px; overflow:auto; padding:4px 0; font:12px/20px ui-monospace, SFMono-Regular, Menlo, monospace; color:#c9d1d9; }
.rui-tree-row { display:flex; align-items:center; gap:6px; padding:0 10px; white-space:nowrap; cursor:default; }
.rui-tree-row.rui-tree-cont { cursor:pointer; }
.rui-tree-row.rui-tree-cont:hover { background:#161b22; }
.rui-tree-arrow { flex:0 0 12px; color:#8b949e; text-align:center; }
.rui-tree-label { color:#79c0ff; }
.rui-tree-prev { color:#8b949e; }
.rui-tree-val { color:#c9d1d9; overflow:hidden; text-overflow:ellipsis; }
/* ui::chart —— Vega-Lite 子集（line/bar/point/area）。坐标全用 %：
   SVG viewBox 0 0 100 100 + preserveAspectRatio=none，描边靠
   vector-effect 保持宽度——无需像素测量，尺寸变化自动跟随。 */
.rui-chart { position:relative; height:240px; background:#0d1117; border:1px solid #30363d; border-radius:6px; overflow:hidden; }
.rui-chart-plot { position:absolute; left:48px; right:14px; top:10px; bottom:26px; }
.rui-chart-grid { position:absolute; left:0; right:0; height:1px; background:#21262d; }
.rui-chart-ytick { position:absolute; right:100%; margin-right:6px; transform:translateY(-50%); color:#8b949e; font-size:11px; white-space:nowrap; }
.rui-chart-xtick { position:absolute; top:100%; margin-top:4px; transform:translateX(-50%); color:#8b949e; font-size:11px; white-space:nowrap; }
.rui-chart-bar { position:absolute; bottom:0; background:#1f6feb; border-radius:1px 1px 0 0; }
.rui-chart-dot { position:absolute; width:5px; height:5px; border-radius:50%; background:#58a6ff; transform:translate(-50%,-50%); }
.rui-chart-svg { position:absolute; left:0; top:0; width:100%; height:100%; overflow:visible; }
.rui-chart-line { fill:none; stroke:#58a6ff; stroke-width:1.5; }
.rui-chart-area { fill:rgba(88,166,255,0.18); stroke:none; }
.rui-chart-empty { position:absolute; inset:0; display:flex; align-items:center; justify-content:center; color:#8b949e; font-size:12px; }
`;
  document.head.appendChild(style);
}
