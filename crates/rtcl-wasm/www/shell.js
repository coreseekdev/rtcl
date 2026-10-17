// shell.js — web 终端组件（从 index.html 提取为可复用模块）
//
// 行编辑/历史/多行续读/粘贴，语义与 tclsh 交互模式对齐：结果裸回显，
// 错误打裸消息；完整性判定走解释器自己的 rtcl_parser::is_complete，
// 永不与真正的 Tcl reader 漂移。

export function createShell(term, { exec, isComplete }) {
  term.classList.add('rsh-term');
  const out = document.createElement('div');
  const activeLine = document.createElement('div');
  activeLine.id = 'rsh-active-line';
  const ps = document.createElement('span');
  ps.className = 'rsh-prompt';
  ps.textContent = '%';
  const mirror = document.createElement('span');
  mirror.id = 'rsh-mirror';
  activeLine.append(ps, document.createTextNode(' '), mirror);
  const kbd = document.createElement('input');
  kbd.id = 'rsh-kbd';
  kbd.autocomplete = 'off';
  kbd.autocapitalize = 'off';
  kbd.spellcheck = false;
  term.append(out, activeLine, kbd);
  injectShellStyles();

  let buf = '';        // current input line
  let pending = '';    // accumulated lines of an incomplete command
  let history = [];    // submitted commands, newest last
  let histIdx = -1;

  function print(text, cls) {
    const div = document.createElement('div');
    div.className = 'rsh-line' + (cls ? ' rsh-' + cls : '');
    div.textContent = text;
    out.appendChild(div);
    term.scrollTop = term.scrollHeight;
  }

  function render() {
    const pos = kbd.selectionStart ?? buf.length;
    const before = buf.slice(0, pos);
    const at = buf.slice(pos, pos + 1) || ' ';
    const after = buf.slice(pos + 1);
    mirror.textContent = '';
    mirror.append(before);
    const caret = document.createElement('span');
    caret.id = 'rsh-caret';
    caret.textContent = at;
    mirror.append(caret);
    mirror.append(after);
  }

  function submit() {
    const line = buf;
    buf = '';
    kbd.value = '';
    print(ps.textContent + ' ' + line, 'echo');
    if (line.trim()) {
      if (history[history.length - 1] !== line) history.push(line);
      if (history.length > 500) history.shift();
    }
    histIdx = -1;

    pending += (pending ? '\n' : '') + line;
    if (!isComplete(pending)) {
      ps.textContent = '...';   // tclsh 无续行提示符；rtcl 标出（CLI 同）
      render();
      return;
    }

    const script = pending;
    pending = '';
    ps.textContent = '%';
    if (script.trim()) run(script);
    render();
  }

  // 输出对齐 tclsh 交互模式：结果裸回显，错误打裸消息（无前缀——
  // errorInfo 栈是脚本模式行为），顶层 return 回显其值，
  // break/continue 报 tclsh 的异常文案，exit 标注退出码。
  function run(script) {
    try {
      const result = exec(script);
      if (result !== '') print(result, 'result');
    } catch (e) {
      const msg = String(e && e.message || e);
      let m;
      if ((m = msg.match(/^return with value: (.*)$/s))) {
        if (m[1] !== '') print(m[1], 'result');
      } else if (msg === 'break') {
        print('invoked "break" outside of a loop', 'error');
      } else if (msg === 'continue') {
        print('invoked "continue" outside of a loop', 'error');
      } else if ((m = msg.match(/^exit with value: (\d+)$/))) {
        print('exit ' + m[1], 'hint');
      } else {
        print(msg, 'error');
      }
    }
  }

  function historyNav(dir) {
    if (history.length === 0) return;
    if (dir < 0) {
      if (histIdx === -1) histIdx = history.length - 1;
      else if (histIdx > 0) histIdx--;
    } else {
      if (histIdx === -1) return;
      histIdx++;
      if (histIdx >= history.length) histIdx = -1;
    }
    buf = histIdx === -1 ? '' : history[histIdx];
    kbd.value = buf;
    kbd.setSelectionRange(buf.length, buf.length);
    render();
  }

  kbd.addEventListener('input', () => { buf = kbd.value; render(); });
  kbd.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); submit(); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); historyNav(-1); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); historyNav(1); }
    else if (e.key === 'Tab') { e.preventDefault(); }
  });
  document.addEventListener('selectionchange', () => {
    if (document.activeElement === kbd) render();
  });
  term.addEventListener('click', () => kbd.focus());
  kbd.addEventListener('paste', (e) => {
    const text = (e.clipboardData || window.clipboardData).getData('text');
    if (!text.includes('\n')) return;              // 单行：默认插入
    e.preventDefault();
    print(ps.textContent + ' ' + text.replace(/\n/g, '\n  '), 'echo');
    run(text.replace(/\n+$/, ''));
  });

  render();
  return { print, focus: () => kbd.focus() };
}

let stylesInjected = false;
function injectShellStyles() {
  if (stylesInjected) return;
  stylesInjected = true;
  const style = document.createElement('style');
  style.textContent = `
.rsh-term { overflow-y:auto; padding:12px 16px; cursor:text; line-height:1.5; font-size:13px; font-family:"SF Mono",Consolas,"Liberation Mono",Menlo,monospace; background:#0d1117; color:#c9d1d9; position:relative; }
.rsh-line { white-space:pre-wrap; word-break:break-all; }
.rsh-echo { color:#8b949e; }
.rsh-echo .rsh-prompt-text, .rsh-echo { }
.rsh-result { color:#7ee787; }
.rsh-puts { color:#c9d1d9; }
.rsh-error { color:#f85149; }
.rsh-hint { color:#484f58; }
.rsh-prompt { color:#1f6feb; user-select:none; }
#rsh-active-line { display:flex; }
#rsh-mirror { white-space:pre-wrap; word-break:break-all; }
#rsh-caret { display:inline-block; width:0.6em; border-bottom:2px solid #7ee787; animation:rsh-blink 1.1s step-end infinite; }
@keyframes rsh-blink { 50% { border-color:transparent; } }
#rsh-kbd { position:absolute; left:-9999px; width:10px; opacity:0; }
`;
  document.head.appendChild(style);
}
