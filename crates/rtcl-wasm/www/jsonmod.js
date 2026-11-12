// jsonmod.js — rtcl `json` 子系统的 JS 外置实现（§10quater 首个命令族
// 外置）。与 native json.rs 同面：
//
//   json decode  ?-index? ?-null string? ?-schema? json-string
//   json encode  value ?schema?
//
// JSON tokenizer 保留数字原文（JSON.parse 会把 1.50 变 1.5，native 是
// 文本保留）；Tcl 序列化对齐 rtcl 的 serialize_list/tcl_quote 规则
// （needsQuote/balanced/backslash-quote）；Tcl list 解析（{} 引用 /
// 引号 / 反斜杠）服务 encode 的值输入面。schema 语法同 parse_schema：
// str/num/bool/obj/list/mixed（brace words 支持）。
//
// decode 返回：Tcl 值字符串（对象→键值对 list、数组→list、null→
// -null 值）；-schema 时返回 [值字符串, schema 字符串] 二元组。
// encode 输入：Tcl 值字符串 + schema 文本；键排序输出（native 同）。

const NEEDS_QUOTE = /[\s"{}[\]$;\\]/;

function needsQuote(s) {
  return s === '' || NEEDS_QUOTE.test(s) || s.startsWith('#');
}

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

function serializeList(items) {
  return items.map(quote).join(' ');
}

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
        else if (c === '}') {
          depth--; i++;
          if (depth === 0) break;
          word += c;
        } else { word += c; i++; }
      }
      out.push(word);
    } else if (s[i] === '"') {
      i++;
      let word = '';
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

// ── JSON tokenizer（数字原文保留）────────────────────────────────

function parseJson(s) {
  let pos = 0;
  const ws = () => { while (pos < s.length && /[ \t\n\r]/.test(s[pos])) pos++; };
  const peek = () => { ws(); return s[pos]; };
  const expect = (c) => {
    if (s[pos] !== c) throw new Error('invalid JSON string');
    pos++;
  };
  const lit = (w) => {
    if (s.startsWith(w, pos)) pos += w.length;
    else throw new Error('invalid JSON string');
  };
  function number() {
    const start = pos;
    while (pos < s.length && /[+\-0-9.eE]/.test(s[pos])) pos++;
    const t = s.slice(start, pos);
    if (t === '' || !/^[+-]?([0-9]+\.?[0-9]*|\.[0-9]+)([eE][+-]?[0-9]+)?$/.test(t)) {
      throw new Error('invalid JSON number');
    }
    return t;
  }
  function string() {
    expect('"');
    let out = '';
    while (pos < s.length && s[pos] !== '"') {
      const c = s[pos];
      if (c === '\\') {
        pos++;
        const e = s[pos++];
        if (e === 'u') {
          out += String.fromCharCode(parseInt(s.slice(pos, pos + 4), 16));
          pos += 4;
        } else if (e === 'b') out += '\b';
        else if (e === 'f') out += '\f';
        else out += { n: '\n', t: '\t', r: '\r', '"': '"', '\\': '\\', '/': '/' }[e] ?? e;
      } else { out += c; pos++; }
    }
    expect('"');
    return out;
  }
  function object() {
    expect('{');
    const entries = [];
    ws();
    if (peek() === '}') { pos++; return { obj: entries }; }
    for (;;) {
      ws();
      const key = string();
      ws(); expect(':');
      entries.push([key, value()]);
      ws();
      if (peek() === ',') { pos++; continue; }
      if (peek() === '}') { pos++; break; }
      throw new Error('invalid JSON string');
    }
    return { obj: entries };
  }
  function array() {
    expect('[');
    const items = [];
    ws();
    if (peek() === ']') { pos++; return { arr: items }; }
    for (;;) {
      items.push(value());
      ws();
      if (peek() === ',') { pos++; continue; }
      if (peek() === ']') { pos++; break; }
      throw new Error('invalid JSON string');
    }
    return { arr: items };
  }
  function value() {
    const c = peek();
    if (c === '{') return object();
    if (c === '[') return array();
    if (c === '"') return { str: string() };
    if (c === 't') { lit('true'); return { num: 'true' }; }
    if (c === 'f') { lit('false'); return { num: 'false' }; }
    if (c === 'n') { lit('null'); return { num: 'null' }; }
    if (c === 'I') { lit('Infinity'); return { num: 'Inf' }; }
    if (c === 'N') { lit('NaN'); return { num: 'NaN' }; }
    if (c === '-') {
      if (s[pos + 1] === 'I') { lit('-Infinity'); return { num: '-Inf' }; }
      return { num: number() };
    }
    if (/[+0-9]/.test(c || '')) return { num: number() };
    throw new Error('invalid JSON: unexpected character ' + (c || 'EOF'));
  }
  ws();
  if (pos >= s.length) throw new Error('empty JSON string');
  const c = peek();
  if (c !== '{' && c !== '[') {
    throw new Error('root element must be an object or an array');
  }
  return value();
}

// ── JSON → Tcl 值字符串 ─────────────────────────────────────────

function toTcl(v, indexMode, nullValue) {
  if (v && v.num !== undefined) {
    let t = v.num;
    if (t === 'null') t = nullValue;
    return t;
  }
  if (v && v.str !== undefined) return v.str;
  if (v && v.arr !== undefined) {
    const items = v.arr.map((x) => toTcl(x, false, nullValue));
    if (indexMode) {
      // 索引 dict：键值对是两个独立元素（0 10 1 20）
      const parts = [];
      items.forEach((x, i) => { parts.push(String(i)); parts.push(quote(x)); });
      return parts.join(' ');
    }
    return serializeList(items);
  }
  if (v && v.obj !== undefined) {
    // 键值对是两个独立元素（native dict 文本形：a 1 b {2 3}）——
    // 值侧走 quote（"{2 3}" / 裸词），不是整对 quote。
    const parts = [];
    for (const [k, x] of v.obj) {
      parts.push(quote(k));
      parts.push(quote(toTcl(x, false, nullValue)));
    }
    return parts.join(' ');
  }
  return '';
}

function buildSchema(v, indexMode) {
  if (v && v.obj !== undefined) {
    let s = 'obj';
    for (const [k, x] of v.obj) {
      const vs = buildSchema(x, false);
      s += ' ' + k + ' ' + (vs.includes(' ') ? '{' + vs + '}' : vs);
    }
    return s;
  }
  if (v && v.arr !== undefined) return 'list';
  if (v && v.num !== undefined) {
    return (v.num === 'true' || v.num === 'false') ? 'bool' : 'num';
  }
  return 'str';
}

// ── Tcl 字符串 → 结构（encode 输入面）───────────────────────────

function tclToValue(s, schema, seen) {
  const scalar = (t) => {
    if (t === 'str') return s;
    if (t === 'num') {
      const trimmed = s.trim();
      if (trimmed === 'Inf' || trimmed === 'inf') return { raw: 'Infinity' };
      if (trimmed === '-Inf' || trimmed === '-inf') return { raw: '-Infinity' };
      const n = Number(trimmed);
      // native encode_num 直接放文本（不校验）——非数字文本保留原样
      return Number.isNaN(n) ? { raw: trimmed } : n;
    }
    // bool：native as_bool/tuthy 规则
    if (s === 'true' || s === '1' || s === 'yes' || s === 'on') return true;
    if (s === 'false' || s === '0' || s === 'no' || s === 'off' || s === '') return false;
    return true;
  };
  if (schema.t === 'str') return scalar('str');
  if (schema.t === 'num') return scalar('num');
  if (schema.t === 'bool') return scalar('bool');
  if (schema.t === 'list') {
    return parseTclList(s).map((it) => tclToValue(it, schema.elem, seen));
  }
  if (schema.t === 'obj') {
    const items = parseTclList(s);
    const pairs = [];
    for (let i = 0; i + 1 < items.length; i += 2) pairs.push([items[i], items[i + 1]]);
    const named = new Map(schema.fields.map((f) => [f.name, f.schema]));
    const wildcard = schema.fields.find((f) => f.name === '*');
    const keys = pairs.map(([k]) => k).sort();
    const out = {};
    for (const k of keys) {
      const v = pairs.find(([pk]) => pk === k)[1];
      const fs = named.get(k) ?? (wildcard ? wildcard.schema : null);
      // 无字段 schema：值为 str（native encode_object 同——字段缺失时
      // 默认 str，非递归解析）
      out[k] = fs ? tclToValue(v, fs, seen) : v;
    }
    return out;
  }
  if (schema.t === 'mixed') {
    const items = parseTclList(s);
    if (items.length !== schema.parts.length) {
      throw new Error('mixed schema: element count mismatch');
    }
    return items.map((it, i) => tclToValue(it, schema.parts[i], seen));
  }
  return s;
}

function parseSchema(s) {
  s = (s || 'str').trim();
  if (s === '' || s === 'str') return { t: 'str' };
  if (s === 'num') return { t: 'num' };
  if (s === 'bool') return { t: 'bool' };
  const sp = s.indexOf(' ');
  const kw = sp < 0 ? s : s.slice(0, sp);
  const rest = sp < 0 ? '' : s.slice(sp + 1).trim();
  if (kw === 'obj') return { t: 'obj', fields: parseSchemaFields(rest) };
  if (kw === 'list') return { t: 'list', elem: rest ? parseSchema(rest) : { t: 'str' } };
  if (kw === 'mixed') return { t: 'mixed', parts: parseTclList(rest).map(parseSchema) };
  return { t: 'str' };
}

function parseSchemaFields(s) {
  const words = parseTclList(s);
  const fields = [];
  for (let i = 0; i + 1 < words.length; i += 2) {
    fields.push({ name: words[i], schema: parseSchema(words[i + 1]) });
  }
  return fields;
}

// JSON 输出（对齐 native：obj 成员 ": " 与 ", " 分隔；str 走
// encode_string 的转义表）。
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

function jsonStringify(v) {
  if (v && typeof v === 'object' && v.raw !== undefined && Object.keys(v).length === 1) {
    return String(v.raw);
  }
  if (v === null || v === undefined) return String(v);
  if (typeof v === 'string') return jsonEscapeStr(v);
  if (typeof v === 'number') return String(v);
  if (typeof v === 'boolean') return String(v);
  if (Array.isArray(v)) return '[' + v.map(jsonStringify).join(', ') + ']';
  const keys = Object.keys(v).sort();
  return '{' + keys.map((k) => jsonEscapeStr(k) + ': ' + jsonStringify(v[k])).join(', ') + '}';
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

export function makeJsonMod() {
  return {
    decode(...rest) {
      const args = normalizeArgs(rest);
      let indexMode = false, nullValue = 'null', schemaMode = false, jsonStr = null;
      for (let i = 0; i < args.length; i++) {
        if (args[i] === '-index') indexMode = true;
        else if (args[i] === '-null') nullValue = args[++i];
        else if (args[i] === '-schema') schemaMode = true;
        else { jsonStr = args[i]; break; }
      }
      if (jsonStr === null || jsonStr === '') throw new Error('empty JSON string');
      const v = parseJson(jsonStr);
      const text = toTcl(v, indexMode, nullValue);
      if (schemaMode) return [text, buildSchema(v, indexMode)];
      return text;
    },
    encode(...rest) {
      const args = normalizeArgs(rest);
      const value = args[0];
      const schemaStr = args[1] || 'str';
      const schema = parseSchema(schemaStr);
      const v = tclToValue(value, schema, new Set());
      // native encode_object/encode_list 的空格风格：{"k": v, "k2": v2}
      // / [v1, v2]（冒号/逗号后单空格；数组内无冒号）。
      return jsonStringify(v);
    },
  };
}
