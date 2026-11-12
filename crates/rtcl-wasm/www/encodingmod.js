// encodingmod.js — rtcl `encoding` 命令面的 JS 外置实现
//（wasm 构建不含 native encoding 命令；TextDecoder/TextEncoder 数据面）。
//
// 命令面（native io.rs cmd_encoding 同形）：
//   encoding convertfrom ?-strict? encodingName data
//   encoding convertto   ?-strict? encodingName string
//   encoding names       → 86 名宽表（与 native NAMES 一致）
//   encoding system      → utf-8（rtcl 固定；wasm 不支持改名）
//   encoding dirs        → 空列表（native 为 icu 路径面）
//
// 字节串约定：latin-1 字符串（每字节 = U+0000..U+00FF）——与 rtcl 的
// byte_str 语义一致。JS 端经 Uint8Array 桥接 TextDecoder/TextEncoder。
//
// 86 名 → WHATWG label 映射；不支持的（symbol/dingbats/mac 符号字体/
// iso2022 系/ebcdic/identity 等）convert 时如实报错
//（"unsupported encoding"），names 仍列全表（native 同）。
//
// 语义子集边界（DIVERGENCES）：-strict 与替换字符行为按 WHATWG 容错
// 替换近似；未映射名在 native 有表实现——全集仍在 native 构建。

const WHATWG = {
  // ── 单字节/Windows ──
  'ascii': 'windows-1252',          // Tcl ascii 只 0-127：高字节容错替换近似
  'iso8859-1': 'windows-1252',
  'latin1': 'windows-1252',
  'iso8859-2': 'iso-8859-2', 'iso8859-3': 'iso-8859-3', 'iso8859-4': 'iso-8859-4',
  'iso8859-5': 'iso-8859-5', 'iso8859-6': 'iso-8859-6', 'iso8859-7': 'iso-8859-7',
  'iso8859-8': 'iso-8859-8', 'iso8859-9': 'windows-1254', 'iso8859-10': 'iso-8859-10',
  'iso8859-11': 'windows-874', 'iso8859-13': 'windows-1257', 'iso8859-14': 'iso-8859-14',
  'iso8859-15': 'iso-8859-15', 'iso8859-16': 'iso-8859-16',
  'cp437': 'ibm437', 'cp850': 'ibm850', 'cp852': 'ibm852', 'cp855': 'ibm855',
  'cp857': 'ibm857', 'cp860': 'ibm860', 'cp861': 'ibm861', 'cp862': 'ibm862',
  'cp863': 'ibm863', 'cp864': 'ibm864', 'cp865': 'ibm865', 'cp866': 'ibm866',
  'cp869': 'ibm869', 'cp874': 'windows-874', 'cp737': null, 'cp775': 'ibm775',
  'cp1250': 'windows-1250', 'cp1251': 'windows-1251', 'cp1252': 'windows-1252',
  'cp1253': 'windows-1253', 'cp1254': 'windows-1254', 'cp1255': 'windows-1255',
  'cp1256': 'windows-1256', 'cp1257': 'windows-1257', 'cp1258': 'windows-1258',
  'koi8-r': 'koi8-r', 'koi8-u': 'koi8-u', 'koi8-t': null, 'koi8-ru': null,
  // ── CJK ──
  'shiftjis': 'shift_jis', 'cp932': 'shift_jis', 'macJapan': null,
  'big5': 'big5', 'cp950': 'big5',
  'gb2312': 'gb18030', 'gb12345': 'gb18030', 'cp936': 'gb18030', 'gb1988': null,
  'gb2312-raw': null, 'cp949': 'euc-kr', 'ksc5601': null, 'euc-kr': 'euc-kr',
  'euc-cn': 'gb18030', 'euc-jp': 'euc-jp',
  'tis-620': 'windows-874',
  // ── Unicode 族 ──
  'utf-8': 'utf-8', 'unicode': 'utf-16le', 'cp1258': 'windows-1258',
  // ── Mac ──（WHATWG 仅 macintosh/x-mac-cyrillic）
  'macRoman': 'macintosh', 'macCroatian': null, 'macRomania': null,
  'macCyrillic': 'x-mac-cyrillic', 'macUkraine': null, 'macGreek': null,
  'macIceland': null, 'macTurkish': null, 'macThai': null,
  'macCentEuro': null, 'macDingbats': null,
  // ── 符号字体/ISO2022/EBCDIC/identity：WHATWG 不支持 ──
  'symbol': null, 'dingbats': null,
  'iso2022': null, 'iso2022-jp': null, 'iso2022-kr': null,
  'ebcdic': null, 'identity': 'identity',
  'jis0201': null, 'jis0208': null, 'jis0212': null, 'cns11643': null,
};

// native NAMES 宽表顺序（86 名；encoding names 输出顺序与 native 一致）
const NAMES = [
  'cp860', 'cp861', 'cp862', 'cp863', 'tis-620', 'cp864', 'cp865', 'cp866', 'gb12345',
  'gb2312-raw', 'cp949', 'cp950', 'cp869', 'dingbats', 'ksc5601', 'macCentEuro', 'cp874',
  'cp165', 'macUkraine', 'jis0201', 'gb2312', 'euc-cn', 'euc-jp', 'macThai', 'iso8859-10',
  'iso2022-jp', 'jis0208', 'iso8859-11', 'iso2022', 'macIceland', 'iso8859-13', 'jis0212',
  'iso8859-14', 'iso8859-15', 'cp737', 'iso8859-16', 'euc-kr', 'big5', 'macRomania',
  'iso2022-kr', 'gb1988', 'macTurkish', 'macGreek', 'ascii', 'cp437', 'macRoman',
  'iso8859-1', 'iso8859-2', 'iso8859-3', 'koi8-r', 'macCroatian', 'ebcdic', 'iso8859-4',
  'iso8859-5', 'cp1250', 'koi8-t', 'cp1251', 'iso8859-6', 'macCyrillic', 'koi8-u', 'cp1252',
  'iso8859-7', 'macDingbats', 'cp1253', 'iso8859-8', 'iso8859-9', 'koi8-ru', 'cp1254',
  'cp850', 'cp1255', 'cp932', 'cp1256', 'identity', 'cp1257', 'cp852', 'cns11643',
  'macJapan', 'cp1258', 'utf-8', 'shiftjis', 'cp936', 'cp855', 'cp775', 'symbol', 'cp857',
  'unicode',
];

function resolveLabel(name) {
  const label = WHATWG[name.toLowerCase()];
  if (!label) throw new Error('unsupported encoding "' + name + '" in this build');
  if (label === 'identity') return null;   // identity = 直通
  return label;
}

function bytesFromByteStr(s) {
  const u = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i) & 0xff;
  return u;
}
function byteStrFromBytes(u) {
  let s = '';
  for (let i = 0; i < u.length; i++) s += String.fromCharCode(u[i]);
  return s;
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

export function makeEncodingMod() {
  return {
    encoding(...rest) {
      const args = normalizeArgs(rest);
      // args: [subcommand, ...rest]（join 协议拆分后的原词）
      const sub = args[0];
      if (sub === 'names') return NAMES.join(' ');
      if (sub === 'dirs') return '';
      if (sub === 'system') {
        // rtcl 固定 utf-8；不接受改名（wasm 面）
        return 'utf-8';
      }
      if (sub === 'convertfrom' || sub === 'convertto') {
        let i = 1;
        let strict = false;
        if (args[i] === '-strict') { strict = true; i++; }
        const name = args[i++];
        const data = args[i];
        if (name === undefined || data === undefined) {
          throw new Error('wrong # args: should be "encoding ' + sub + ' encodingName data"');
        }
        const label = resolveLabel(name);
        if (sub === 'convertfrom') {
          // data = 字节串（latin-1 字符）；TextDecoder 解码为文本
          if (label === null) return data;           // identity 直通
          const u = bytesFromByteStr(data);
          const dec = new TextDecoder(label, { fatal: strict });
          return dec.decode(u);
        }
        // convertto：文本 → 字节串
        if (label === null) return data;
        if (label === 'utf-8') {
          const u = new TextEncoder().encode(data);
          return byteStrFromBytes(u);
        }
        const enc = new TextEncoder();               // 仅 utf-8——其余走 decode 反表
        // TextEncoder 只支持 utf-8：非 utf-8 用「utf-8 解码对照」的
        // 反查表（构造 label→char 映射一次）。
        const rev = reverseTable(label);
        let out = '';
        for (const ch of data) {
          const b = rev.get(ch);
          if (b === undefined) {
            if (strict) throw new Error("couldn't encode \"" + data + '": character out of range');
            out += '?';
          } else out += String.fromCharCode(b);
        }
        return out;
      }
      throw new Error('unknown or ambiguous subcommand "' + sub +
        '": must be convertfrom, convertto, dirs, names, or system');
    },
  };
}

// 反查表缓存：label → {char: byte}
const _revCache = new Map();
function reverseTable(label) {
  let m = _revCache.get(label);
  if (m) return m;
  m = new Map();
  // 用 utf-8 正反构造：对 0..255 每字节以 label 解码，得该字节代表的字符
  for (let b = 0; b < 256; b++) {
    try {
      const dec = new TextDecoder(label, { fatal: true });
      const ch = dec.decode(new Uint8Array([b]));
      // 多字节序列（如 gb18030 lead byte）跳过——完整序列转换走
      // TextDecoder 主路径（convertfrom）；encode 侧多字节输出近似以
      // 首字符映射。
      if (ch.length === 1 && !m.has(ch)) m.set(ch, b);
    } catch { /* 该字节非合法序列起点 */ }
  }
  _revCache.set(label, m);
  return m;
}
