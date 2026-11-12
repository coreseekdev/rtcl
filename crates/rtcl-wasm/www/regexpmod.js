// regexpmod.js — rtcl `regexp`/`regsub` 子系统的 JS 外置实现
//（§10quater 外置第二刀；wasm 构建不含 regexp-lite）。
//
// 命令面与 native regexp_cmds.rs 同形；Tcl ARE → JS RegExp 为**语义
// 子集**（POSIX 类展开、\y→\b、quoting ***=、-nocase/-line/
// -lineanchor/-linestop/-all/-inline/-indices/-start；差异记录
// DIVERGENCES）。
//
// 变量写入协议：matchVar/subMatchVar/regsub varName 在解释器帧上——
// JS 无法直写。带变量名的调用返回**结构化编码**（__RXCAP/__RXIDX/
// __RS 前缀的 Tcl 列表），由页面定义的 ::regexp/::regsub wrapper
// proc 解包并用 upvar 写变量、返回码。无变量调用返回裸结果（0/1/
// inline 列表/count）。

// ── Tcl list 序列化/解析（对齐 serialize_list/tcl_quote）─────────
const NEEDS_QUOTE = /[\s"{}[\]$;\\]/;
function needsQuote(s) { return s === '' || NEEDS_QUOTE.test(s) || s.startsWith('#'); }
function balanced(s) {
  let depth = 0;
  for (const c of s) {
    if (c === '{') depth++;
    else if (c === '}') { depth--; if (depth < 0) return false; }
  }
  return depth === 0;
}
function quote(s) {
  if (!needsQuote(s)) return s;
  if (balanced(s)) return '{' + s + '}';
  let out = '"';
  for (const c of s) {
    if (c === '"' || c === '\\') out += '\\' + c;
    else if (c === '\n') out += '\\n';
    else if (c === '\t') out += '\\t';
    else if (c === '\r') out += '\\r';
    else out += c;
  }
  return out + '"';
}
function serializeList(items) { return items.map(quote).join(' '); }
function parseTclList(s) {
  const out = [];
  let i = 0;
  const n = s.length;
  while (i < n) {
    while (i < n && /\s/.test(s[i])) i++;
    if (i >= n) break;
    if (s[i] === '{') {
      let depth = 1, word = '';
      i++;
      while (i < n) {
        const c = s[i];
        if (c === '{') { depth++; word += c; i++; }
        else if (c === '}') { depth--; i++; if (depth === 0) break; word += c; }
        else { word += c; i++; }
      }
      out.push(word);
    } else if (s[i] === '"') {
      i++; let word = '';
      while (i < n && s[i] !== '"') {
        if (s[i] === '\\' && i + 1 < n) {
          const c = s[i + 1];
          word += c === 'n' ? '\n' : c === 't' ? '\t' : c === 'r' ? '\r' : c;
          i += 2;
        } else word += s[i++];
      }
      i++;
      out.push(word);
    } else {
      let word = '';
      while (i < n && !/\s/.test(s[i])) {
        if (s[i] === '\\' && i + 1 < n) { word += s[i + 1]; i += 2; }
        else word += s[i++];
      }
      out.push(word);
    }
  }
  return out;
}

// ── ARE → JS RegExp 翻译 ─────────────────────────────────────────
const POSIX = {
  alpha: 'A-Za-z', digit: '0-9', alnum: 'A-Za-z0-9', upper: 'A-Z', lower: 'a-z',
  space: ' \\t\\r\\n\\v\\f', blank: ' \\t', punct: '!-/:-@\\[-`{-~',
  xdigit: '0-9A-Fa-f', cntrl: '\\x00-\\x1f\\x7f', graph: '!-~', print: ' -~',
  word: 'A-Za-z0-9_', // [[:word:]]
};

function translateAre(exp) {
  let literal = false;
  let body = exp;
  if (body.startsWith('***=')) { literal = true; body = body.slice(4); }
  let out = '';
  let i = 0;
  const n = body.length;
  let inClass = false;
  while (i < n) {
    const c = body[i];
    if (c === '\\') {
      const e = body[i + 1];
      if (!inClass) {
        if (e === 'y') { out += '\\b'; i += 2; continue; }
        if (e === 'Y') { out += '\\B'; i += 2; continue; }
        if (e === 'm') { out += '\\b(?=[A-Za-z0-9])'; i += 2; continue; }
        if (e === 'M') { out += '\\b(?![A-Za-z0-9])'; i += 2; continue; }
        if (e === '<') { out += '\\b(?=[A-Za-z0-9])'; i += 2; continue; }
        if (e === '>') { out += '(?<=[A-Za-z0-9])\\b'; i += 2; continue; }
      }
      out += c + (e ?? '');
      i += 2;
      continue;
    }
    if (c === '[' && !inClass) {
      // Tcl/POSIX 形 `[[:name:]]`：外层 [ 内嵌 [:name:]——整体替换为
      // JS 类展开；非 POSIX 类则进入普通字符类透传。
      if (body.startsWith('[[:', i)) {
        const end = body.indexOf(':]', i + 3);
        if (end >= 0 && body[end + 2] === ']') {
          const inner = body.slice(i + 3, end);      // 可选 ^ + 类名
          const neg = inner.startsWith('^');
          const name = neg ? inner.slice(1) : inner;
          const t = POSIX[name];
          if (t) {
            out += neg ? '[^' + t + ']' : '[' + t + ']';
            i = end + 3;                             // 跳过 ':]' 与外层 ']'
            continue;
          }
        }
      }
      inClass = true;
      continue;
    }

    if (c === '[') inClass = true;
    else if (c === ']') inClass = false;
    out += c;
    i++;
  }
  return { source: out, literal };
}


// 入口参数规范：dynmod join 协议下可能收到单串（\u0001 分隔的整条
// 命令行）——还原为原词数组。
function normalizeArgs(rest) {
  // JS 直调数组形态（rest[0] 为数组）→ 视作已拆原词
  if (rest.length === 1 && Array.isArray(rest[0])) return rest[0];
  if (rest.length === 1 && typeof rest[0] === 'string' && rest[0].includes('\u0001')) {
    return rest[0].split('\u0001');
  }
  return rest;
}

export function makeRegexpMod() {
  function parseOpts(args, bools, withStart) {
    const opts = { nocase: false, line: false, lineanchor: false, linestop: false,
                   all: false, inline: false, indices: false, about: false, start: 0 };
    let i = 0;
    for (; i < args.length; i++) {
      const a = args[i];
      if (a === '--') { i++; break; }
      if (a.startsWith('-') && a.length > 1) {
        if (bools.includes(a.slice(1))) { opts[a.slice(1)] = true; continue; }
        if (withStart && a === '-start') { opts.start = Number(args[++i]) || 0; continue; }
        break;
      }
      break;
    }
    return { opts, rest: args.slice(i) };
  }

  function compile(exp, opts, sticky) {
    const { source, literal } = translateAre(exp);
    // 统一 g 形：非 -all 调用只取首个 exec（搜索语义，tclsh 同）；
    // -start 经 lastIndex 承载。sticky(y) 会锚定开头——错误。
    void sticky;
    if (literal) {
      let lit = '';
      for (const c of source) if ('\\^$.|?*+()[]{}'.includes(c)) lit += '\\' + c; else lit += c;
      return { re: new RegExp(lit, (opts.nocase ? 'i' : '') + 'g'), literal: true };
    }
    let flags = 'g';
    if (opts.nocase) flags += 'i';
    if (opts.lineanchor || opts.line) flags += 'm';
    return { re: new RegExp(source, flags), literal: false };
  }

  const encodeStr = (s) => jsonEscapeStr(s);

  function jsonEscapeStr(s) {
    let out = '"';
    for (const c of s) {
      if (c === '"') out += '\\"';
      else if (c === '\\') out += '\\\\';
      else if (c === '/') out += '\\/';
      else if (c === '\b') out += '\\b';
      else if (c === '\f') out += '\\f';
      else if (c === '\n') out += '\\n';
      else if (c === '\r') out += '\\r';
      else if (c === '\t') out += '\\t';
      else if (c < ' ') out += '\\u' + c.charCodeAt(0).toString(16).padStart(4, '0');
      else out += c;
    }
    return out + '"';
  }

  return {
    // regexp → 裸结果（0/1、inline 列表、count）或结构化编码
    // {__RXCAP mode vals...}（Tcl 列表字符串，wrapper 解包写变量）
    regexp(...rest) {
      const args = normalizeArgs(rest);

      const { opts, rest: parsed } = parseOpts(args,
        ['nocase', 'line', 'lineanchor', 'linestop', 'all', 'inline', 'indices', 'about'], true);
      const [exp, string, ...capVars] = parsed;
      if (exp === undefined || string === undefined) {
        throw new Error('wrong # args: should be "regexp ?-option ...? exp string ?matchVar? ?subMatchVar ...?"');
      }
      const k = capVars.length;
      const built = compile(exp, opts, !opts.all);
      const re = built.re;
      re.lastIndex = opts.start;

      const flatVals = (m) => {
        const vals = [m[0] ?? ''];
        for (let g = 1; g < m.length; g++) vals.push(m[g] ?? '');
        return vals;
      };
      const flatPairs = (m) => {
        const out = [m.index, m.index + (m[0]?.length ?? 1) - 1];
        for (let g = 1; g < m.length; g++) {
          if (m[g] === undefined) { out.push(-1, -1); continue; }
          const start = m[0].indexOf(m[g]);
          if (start < 0) { out.push(-1, -1); continue; }
          out.push(m.index + start, m.index + start + m[g].length - 1);
        }
        return out;
      };

      if (opts.about) {
        const about = [exp];
        if (/^\^/.test(exp)) about.push('anchor-start');
        if (/\$$/.test(exp)) about.push('anchor-end');
        return serializeList(about);
      }

      if (opts.all && opts.inline && k === 0) {
        const out = [];
        let m;
        while ((m = re.exec(string)) !== null) {
          out.push(...flatVals(m).map((v) => quote(v)));
          if (m[0] === '') re.lastIndex++;
        }
        return out.length ? out.join(' ') : '';
      }
      if (opts.all && opts.indices && k === 0) {
        const out = [];
        let m;
        while ((m = re.exec(string)) !== null) {
          const p = flatPairs(m);
          out.push('{' + p[0] + ' ' + p[1] + '}');
          for (let g = 2; g < p.length; g += 2) out.push('{' + p[g] + ' ' + p[g + 1] + '}');
          if (m[0] === '') re.lastIndex++;
        }
        return out.join(' ');
      }
      if (opts.all) {
        let count = 0;
        let m;
        while ((m = re.exec(string)) !== null) {
          count++;
          if (m[0] === '') re.lastIndex++;
        }
        return String(count);
      }

      const m = re.exec(string);
      if (!m) return '0';
      if (k === 0) {
        if (opts.inline) {
          return flatVals(m).map((v) => quote(v)).join(' ');
        }
        if (opts.indices) {
          const p = flatPairs(m);
          return '{' + p[0] + ' ' + p[1] + '}';
        }
        return '1';
      }
      // 带变量：结构化编码（wrapper 写变量）
      if (opts.indices) {
        const p = flatPairs(m);
        const parts = ['__RXIDX', String(k)];
        for (let g = 0; g < p.length; g += 2) parts.push(quote('{' + p[g] + ' ' + p[g + 1] + '}'));
        return parts.join(' ');
      }
      const vals = flatVals(m);
      const parts = ['__RXCAP', String(k)];
      for (const v of vals) parts.push(quote(v));
      return parts.join(' ');
    },

    // regsub → 裸 count（无 varName）或 {__RS count result} 编码
    probe(...args) {
      return JSON.stringify({ n: args.length, a: args });
    },

    regsub(...rest) {
      const args = normalizeArgs(rest);
      const { opts, rest: parsed } = parseOpts(args,
        ['nocase', 'all', 'line', 'lineanchor', 'linestop'], true);
      const [exp, string, subSpec, varName] = parsed;
      if (exp === undefined || string === undefined || subSpec === undefined) {
        throw new Error('wrong # args: should be "regsub ?-option ...? exp string subSpec varName"');
      }
      const built = compile(exp, opts, !opts.all);
      const re = built.re;
      re.lastIndex = opts.start;
      const expand = (spec, m) => {
        let out = '';
        let i = 0;
        while (i < spec.length) {
          const c = spec[i];
          if (c === '&' && (i === 0 || spec[i - 1] !== '\\')) { out += m[0] ?? ''; i++; continue; }
          if (c === '\\' && i + 1 < spec.length) {
            const nx = spec[i + 1];
            if (nx >= '0' && nx <= '9') { out += m[Number(nx)] ?? ''; i += 2; continue; }
            if (nx === '\\') { out += '\\'; i += 2; continue; }
          }
          out += c;
          i++;
        }
        return out;
      };
      let count = 0;
      let result = '';
      if (opts.all) {
        let last = 0;
        let m;
        while ((m = re.exec(string)) !== null) {
          result += string.slice(last, m.index);
          result += expand(subSpec, m);
          last = m.index + m[0].length;
          count++;
          if (m[0] === '') re.lastIndex++;
        }
        result += string.slice(last);
      } else {
        const m = re.exec(string);
        if (!m) return '0';
        result = string.slice(0, m.index) + expand(subSpec, m) + string.slice(m.index + m[0].length);
        count = 1;
      }
      if (varName === undefined) return String(count);
      return serializeList(['__RS', String(count), result]);
    },
  };
}
