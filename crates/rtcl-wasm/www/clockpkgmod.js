// clockpkgmod.js — rtcl `clock` + `package` 命令面的 JS 外置实现
//（wasm 构建剥离 clock/package feature 后的补回面；方案 a = JS dynmod，
// 2026-11-19 用户拍板——命令计数面零变化：JS 面注册同面命令补回）。
//
// 命令面（native clock.rs / package.rs 同形，D-24/D-25 收口态移植）：
//   clock seconds|clicks|microseconds|milliseconds|format|scan|add
//     （子命令 Tcl_GetIndexFromObj 语义：精确/唯一前缀/歧义报错）
//   package provide|require|names|forget|vcompare|vsatisfies
//     （require 的 pkgIndex 扫描由页面 wrapper proc 在 Tcl 面编排：
//      JS 命令回调无法重入解释器 source——registry 状态在 JS，
//      auto_path/pkgIndex 流程在 CLOCKPKG_SETUP_TCL，语义与 native
//      pkg_require 逐步同构）
//
// 时区能力（对 wasm UTC-only stub 的**功能升级**，方案 a 的额外收益）：
//   * 默认渲染 = 宿主本地时区（Date 偏移字段，DST/历史转换由引擎
//     tzdata 承担——native unix 面的 libc localtime_r 同源数据）；
//     wasm stub 恒 UTC（libc 不进 wasm，local_offset_name 恒 None）。
//   * -timezone 支持 IANA 名：Intl.DateTimeFormat timeZone 校验与
//     求值（wasm stub 无 zoneinfo 面，-timezone 恒报 not found；
//     native unix 面 = /usr/share/zoneinfo 同名表）。
//   * %Z = Intl short 名（"UTC"/"GMT+8" 风格，非 libc 缩写表——
//     DIVERGENCES 定调）；-gmt 恒 "GMT"；%z = +HHMM。
//   * DST 消解：fold（秋拨 ambiguity）取转换前偏移、gap（春拨）落
//     转换后墙钟——与 glibc mktime(isdst=-1) 的实测语义一致
//    （native 记录：02:30 春季跳变 +1d → 03:30）。
//
// 行为矩阵（clock.rs 逐条移植）：
//   * format 规定符全集：%Y %y %m %d %e %H %I %M %S %p %P %a %A %b
//     %h %B %j %w %Z %z %n %t %% %s %c %D %T；未知 = 原样 "%x"。
//   * scan -format（strptime 面）：数字字段跳前导空白 1-2 位（%Y 恰
//     4 位、%j 1-3 位）；格式空白须有输入空白；缺日期字段取 -base、
//     时间默认 0；年只在月+日（或 %j）同现生效；%y 窗口 00-37→20xx、
//     38-99→19xx；%p 换算；%z 回推；%Z 缩写表优先于 -gmt/-timezone；
//     %s 直接取 epoch；日期归一化（Feb 30 → Mar 1、13 月 → 次年 1 月）。
//   * 自由 scan（ClockFreeScan 常用子集 + 怪值收口）：空串/now/today/
//     tomorrow/yesterday；ISO/YYYYMMDD/M/D/Y/Y-M-D/Mon D/D Mon/
//     D-Mon-YYYY；时刻 H:MM[:SS][am|pm]/3pm/3 pm/裸数字（≤4 位 HHMM、
//     无逗号 4 位月名后 = 20:00 怪值）；星期名（next/last/Plain）；
//     相对短语（<n> <unit>[s] [ago]、next|last <unit>、裸单位 = +1）；
//     数字时区 ±HHMM、UTC/GMT、军事单字母、TimezoneTable 全表、
//     next|last <月名>（last = (年,月,1) 前推 2 天 + 年份 %y 窗口）。
//   * add：单位按序作用；年/月/日/周按目标时区日历运算（月末夹取
//     Jan 31 +1m = Feb 29）；时/分/秒定长秒；-gmt = UTC 固定秒。
//   * 儒略-格里历切换（1582-10-15，JDN 2299161）：scan/format/add
//     的日期换算 < 切换点用儒略公式（0001-01-01 = -62135769600）。
//
// 错误协议：JS 面错误（消息 + errorCode 同 native）经 __CLKERR 哨兵
// 列表回传（__CLKOK 包裹正常值——结果首词歧义消除），页面 wrapper
// proc（CLOCKPKG_SETUP_TCL）解包并以 `return -code error -errorcode`
// 重抛——"JS command '__dynmodv2:…' error:" 派发前缀不外泄。
//
// 语义边界（docs/DIVERGENCES 同步项）：
//   * 错误 errorInfo 帧链含 wrapper proc 帧（tclsh 的 Tcl 实现帧同
//     类有意不匹配——native 亦然，D-25 残差沿用）；
//   * -locale 收值忽略（native 同）；
//   * 超出 Date/Intl 精度域（|epoch| > 2^53 秒级 / BC 年代 wall 场
//     求值）退化为 offset-0 或 epoch 原值——native i64 全域的残差；
//   * %Z 本地缩写风格（Intl short）与 tclsh/libc 缩写表不同文。

// ── 历法换算（clock.rs 纯整数函数逐条移植）────────────────────────

const JDN_POSIX_EPOCH = 2440588;      // 1970-01-01 的 JDN
const GREGORIAN_CHANGE_JDN = 2299161; // 1582-10-15（格里历）切换点

function daysFromCivil(y, m, d) {
  const yy = m <= 2 ? y - 1 : y;
  const era = Math.floor(yy / 400);
  const yoe = yy - era * 400;
  const mp = m > 2 ? m - 3 : m + 9;
  const doy = Math.floor((153 * mp + 2) / 5) + d - 1;
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
  return era * 146097 + doe - 719468;
}

function civilFromDays(z) {
  const t = z + 719468;
  const era = Math.floor(t / 146097);
  const doe = t - era * 146097;
  const yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36524) - Math.floor(doe / 146096)) / 365);
  const y = yoe + era * 400;
  const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100));
  const mp = Math.floor((5 * doy + 2) / 153);
  const d = doy - Math.floor((153 * mp + 2) / 5) + 1;
  const m = mp < 10 ? mp + 3 : mp - 9;
  return [m <= 2 ? y + 1 : y, m, d];
}

function isLeap(y) { return y % 4 === 0 && (y % 100 !== 0 || y % 400 === 0); }

function daysInMonth(y, m) {
  if (m === 2) return isLeap(y) ? 29 : 28;
  return [31, 0, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1] ?? 31;
}

function daysInMonthJulian(y, m) {
  if (m === 2) return Math.floor((((y % 4) + 4) % 4)) === 0 ? 29 : 28;
  return daysInMonth(y, m);
}

function gregorianJdn(y, m, d) { return daysFromCivil(y, m, d) + JDN_POSIX_EPOCH; }

function julianJdn(y, m, d) {
  const ym1 = y - 1;
  let prior = 0;
  for (let mm = 1; mm < m; mm++) prior += daysInMonthJulian(y, mm);
  return 1721424 - 1 + d + prior + 365 * ym1 + Math.floor(ym1 / 4);
}

// 民用历 → JDN：先按格里历算，落在切换点之前改用儒略历；月份先按
// Tcl 方式归约（13 月 → 次年 1 月）。
function jdnFromCivil(y, m, d) {
  const q = Math.floor((m - 1) / 12);
  const yy = y + q;
  const mm = ((m - 1) % 12 + 12) % 12 + 1;
  const greg = gregorianJdn(yy, mm, d);
  return greg < GREGORIAN_CHANGE_JDN ? julianJdn(yy, mm, d) : greg;
}

// JDN → 民用历（切换点之前用儒略历逆式，之后格里历）。
function civilFromJdn(jdn) {
  if (jdn < GREGORIAN_CHANGE_JDN) {
    const c = jdn + 32082;
    const d1 = Math.floor((4 * c + 3) / 1461);
    const e = c - Math.floor(1461 * d1 / 4);
    const m = Math.floor((5 * e + 2) / 153);
    const day = e - Math.floor((153 * m + 2) / 5) + 1;
    const month = m + 3 - 12 * Math.floor(m / 10);
    return [d1 - 4800 + Math.floor(m / 10), month, day];
  }
  return civilFromDays(jdn - JDN_POSIX_EPOCH);
}

function epochFromJdn(jdn, h, mi, s) {
  return (jdn - JDN_POSIX_EPOCH) * 86400 + h * 3600 + mi * 60 + s;
}

// ── 时区上下文（升级面：本地/IANA/UTC 三态）──────────────────────
// ctx = { gmt: bool, zone: string|null }（null = 宿主本地时区）。

const DT_ANSI = 'en-US';
const dtfCache = new Map();
function dtfFor(zone) {
  let d = dtfCache.get(zone);
  if (!d) {
    d = new Intl.DateTimeFormat(DT_ANSI, {
      timeZone: zone, hourCycle: 'h23',
      year: 'numeric', month: '2-digit', day: '2-digit',
      hour: '2-digit', minute: '2-digit', second: '2-digit',
    });
    dtfCache.set(zone, d);
  }
  return d;
}

// zone（IANA 名或 null=本地）在 epochSec 的东向偏移秒。
function zoneOffsetAt(epochSec, ctx) {
  if (ctx.gmt) return 0;
  if (ctx.zone === null) {
    const ms = epochSec * 1000;
    if (!Number.isFinite(ms) || Math.abs(ms) > 8.64e15) return 0;
    return -new Date(ms).getTimezoneOffset() * 60;
  }
  const ms = epochSec * 1000;
  if (!Number.isFinite(ms) || Math.abs(ms) > 8.64e15) return 0;
  const w = zoneWallFields(epochSec, ctx);
  const asUtc = epochFromJdn(gregorianJdn(w.y, w.mo, w.d), w.h, w.mi, w.s);
  return asUtc - epochSec;
}

function zoneWallFields(epochSec, ctx) {
  const parts = dtfFor(ctx.zone).formatToParts(new Date(epochSec * 1000));
  const get = (t) => {
    const p = parts.find((x) => x.type === t);
    return p ? Number(p.value) : 0;
  };
  return { y: get('year'), mo: get('month'), d: get('day'), h: get('hour'), mi: get('minute'), s: get('second') };
}

const wallEq = (w, y, mo, d, h, mi, s) =>
  w.y === y && w.mo === mo && w.d === d && w.h === h && w.mi === mi && w.s === s;

// 民用历字段 → epoch（目标时区的 DST 消解；fold（秋拨双解）取转换
// 前偏移（= 首个/DST 支——tclsh 8.6.17 与 native 干净上下文同值；
// native 的 with_zone 在同进程 gap 例之后会出现 glibc 状态泄漏的反
// 常值，见 DIVERGENCES 2026-11-19 节）、gap（春拨空档）落转换后墙
// 钟（pre-gap 偏移求值 → 渲染 wall+shift；glibc mktime 同）。
function zoneFromCivil(y, mo, d, h, mi, s, ctx) {
  if (ctx.zone === null) {
    // 本地时区：Date 构造器直接消解（0-99 年的 legacy 映射规避——
    // 走 offset-at-instant，与 native pre-1583 臂同形）。
    if (y >= 100) return new Date(y, mo - 1, d, h, mi, s).getTime() / 1000;
    const utc = epochFromJdn(gregorianJdn(y, mo, d), h, mi, s);
    return utc - zoneOffsetAt(utc, ctx);
  }
  const utcGuess = epochFromJdn(gregorianJdn(y, mo, d), h, mi, s);
  const e0 = utcGuess - zoneOffsetAt(utcGuess, ctx);
  const e1 = utcGuess - zoneOffsetAt(e0, ctx);
  if (wallEq(zoneWallFields(e1, ctx), y, mo, d, h, mi, s)) return e1;
  // fold/gap 细化：转换前偏移优先（fold = DST 支），转换后次之。
  const offPre = zoneOffsetAt(e1 - 86400, ctx);
  const candPre = utcGuess - offPre;
  if (wallEq(zoneWallFields(candPre, ctx), y, mo, d, h, mi, s)) return candPre;
  const offPost = zoneOffsetAt(e1 + 86400, ctx);
  const candPost = utcGuess - offPost;
  if (wallEq(zoneWallFields(candPost, ctx), y, mo, d, h, mi, s)) return candPost;
  // gap：两侧都非合法墙钟——落转换后（pre 偏移求值，glibc 同）。
  return utcGuess - offPre;
}

// epoch 的日历分解（native clock_civil：local = epoch + off）。
function clockCivil(epoch, ctx) {
  const local = ctx.gmt ? epoch : epoch + zoneOffsetAt(epoch, ctx);
  const days = Math.floor(local / 86400);
  const secs = local - days * 86400;
  const [y, mo, d] = civilFromJdn(days + JDN_POSIX_EPOCH);
  return [y, mo, d, Math.floor(secs / 3600), Math.floor((secs % 3600) / 60), secs % 60];
}

// 日历字段 → epoch（native clock_epoch 结构同形：gmt 纯 JDN；现代
// 日期走 zoneFromCivil 的 DST 消解；古日期 offset-at-instant）。
function clockEpoch(y, mo, d, h, mi, s, ctx) {
  if (ctx.gmt) return epochFromJdn(jdnFromCivil(y, mo, d), h, mi, s);
  if (y >= 1583) return zoneFromCivil(y, mo, d, h, mi, s, ctx);
  const utc = epochFromJdn(jdnFromCivil(y, mo, d), h, mi, s);
  return utc - zoneOffsetAt(utc, ctx);
}

// %Z 名：-gmt 恒 "GMT"；否则 Intl short 名（本地 = 无 timeZone）。
function zoneShortName(epochSec, ctx) {
  if (ctx.gmt) return 'GMT';
  try {
    const ms = epochSec * 1000;
    if (!Number.isFinite(ms) || Math.abs(ms) > 8.64e15) return 'UTC';
    const d = new Intl.DateTimeFormat(DT_ANSI, {
      ...(ctx.zone !== null ? { timeZone: ctx.zone } : {}),
      timeZoneName: 'short',
    });
    const p = d.formatToParts(new Date(ms)).find((x) => x.type === 'timeZoneName');
    if (p && p.value) return p.value;
  } catch { /* 求值失败回落 UTC */ }
  return 'UTC';
}

// gmtoff 秒 → "+HHMM"（%z 形状）。
function renderZ(off) {
  const sign = off < 0 ? '-' : '+';
  const a = Math.abs(off);
  return sign + String(Math.floor(a / 3600)).padStart(2, '0') +
    String(Math.floor((a % 3600) / 60)).padStart(2, '0');
}

// ── 名表 / 时区缩写表（native 同源数据）──────────────────────────

const MONTH_NAMES = [
  'january', 'february', 'march', 'april', 'may', 'june',
  'july', 'august', 'september', 'october', 'november', 'december',
];
const WEEKDAY_NAMES = [
  'sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday',
];

// tclsh 8.6.17 TimezoneTable（tclDate.c 同源，以 oracle 实测为准）：
// 缩写（lower）→ off（west 正：epoch = 墙钟按 UTC 解释 + off）。
function zoneAbbrev(lower) {
  const h = 3600;
  const table = {
    gmt: 0, ut: 0, utc: 0, wet: 0,
    bst: -h, wat: h, at: 2 * h,
    nft: 7 * h / 2, nst: 7 * h / 2, ndt: 5 * h / 2,
    ast: 4 * h, adt: 3 * h, est: 5 * h, edt: 4 * h,
    cst: 6 * h, cdt: 5 * h, mst: 7 * h, mdt: 6 * h,
    pst: 8 * h, pdt: 7 * h, yst: 9 * h, ydt: 8 * h,
    akst: 9 * h, akdt: 8 * h, hst: 10 * h, hdt: 9 * h,
    cat: 10 * h, ahst: 10 * h, nt: 11 * h, idlw: 12 * h,
    cet: -h, met: -h, mewt: -h, swt: -h, fwt: -h,
    cest: -2 * h, mest: -2 * h, sst: -2 * h, fst: -2 * h, eet: -2 * h,
    bt: -3 * h, it: -7 * h / 2,
    zp4: -4 * h, zp5: -5 * h, ist: -11 * h / 2, zp6: -6 * h,
    wast: -7 * h, wadt: -8 * h, jt: -15 * h / 2, cct: -8 * h,
    jst: -9 * h, kst: -9 * h, jdt: -10 * h, kdt: -10 * h,
    cast: -19 * h / 2, cadt: -21 * h / 2,
    east: -10 * h, gst: -10 * h, eadt: -21 * h / 2,
    nzt: -12 * h, nzst: -12 * h, idle: -12 * h, nzdt: -13 * h,
  };
  const v = table[lower];
  return v === undefined ? null : v;
}

// 该缩写是否 DST 变体（tDAYZONE 项）。
function zoneIsDst(name) {
  return ['bst', 'edt', 'cdt', 'mdt', 'pdt', 'ydt', 'akdt', 'hdt', 'adt', 'ndt',
    'cest', 'mest', 'sst', 'fst', 'wadt', 'cadt', 'eadt', 'jdt', 'kdt', 'nzdt']
    .includes(name);
}

// %y → 世纪：Tcl 的 32 位 time_t 安全窗。
function mapTwoDigitYear(y) { return y <= 37 ? 2000 + y : 1900 + y; }

// 月份名匹配：全称或缩写前缀（大小写不敏感，已 lower）。
function matchMonthName(w) {
  for (let i = 0; i < MONTH_NAMES.length; i++) {
    if (MONTH_NAMES[i].startsWith(w)) return i + 1;
  }
  return null;
}

// ── 数值/布尔解析（Value::as_int / strict_bool 同形）──────────────

// Tcl 整数字面（十进制/0x/0o/0b/前导 0 历史八进制；超 i64/精度域
// 拒绝——native from_str_radix 溢出拒绝同形）。
function parseTclInt(text) {
  const s = String(text).trim();
  let neg = false;
  let body = s;
  if (body.startsWith('-')) { neg = true; body = body.slice(1); }
  else if (body.startsWith('+')) body = body.slice(1);
  let v = null;
  if (/^0[xX][0-9a-fA-F]+$/.test(body)) v = parseInt(body.slice(2), 16);
  else if (/^0[oO][0-7]+$/.test(body)) v = parseInt(body.slice(2), 8);
  else if (/^0[bB][01]+$/.test(body)) v = parseInt(body.slice(2), 2);
  else if (body.length > 1 && body[0] === '0' && /^[0-7]+$/.test(body)) v = parseInt(body, 8);
  else if (/^[0-9]+$/.test(body)) v = parseInt(body, 10);
  else return null;
  if (v === null || !Number.isFinite(v) || Math.abs(v) > Number.MAX_SAFE_INTEGER) return null;
  return neg ? -v : v;
}

// 严格 Tcl 布尔（scan/add 的 -gmt）：0/1 或六词唯一前缀。
function strictBool(s) {
  if (s === '' || s == null) return null;
  if (s === '0') return false;
  if (s === '1') return true;
  const low = String(s).toLowerCase();
  let hit = null;
  for (const [word, val] of [['false', false], ['true', true], ['no', false],
    ['yes', true], ['off', false], ['on', true]]) {
    if (word.startsWith(low)) {
      if (hit !== null) return null; // 歧义（"o"）
      hit = val;
    }
  }
  return hit;
}

// ── 错误协议（哨兵列表）─────────────────────────────────────────

function clkErr(msg, code) { const e = new Error(msg); e.clkCode = code || ''; throw e; }

// Tcl list 词引用（regexpmod 同款：bare / {…} / "…" 三态）。
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
function tclQuote(s) {
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
const listWords = (items) => items.map(tclQuote).join(' ');

// ── clock format（native format_timestamp 移植）──────────────────

// epoch 的日历分解（含儒略切换；julian 闰年规则）。
function epochToDatetime(epoch) {
  const days = Math.floor(epoch / 86400);
  const daySecs = epoch - days * 86400;
  const hour = Math.floor(daySecs / 3600);
  const minute = Math.floor((daySecs % 3600) / 60);
  const second = daySecs % 60;
  const jdn = days + JDN_POSIX_EPOCH;
  const [year, m, d] = civilFromJdn(jdn);
  const julian = jdn < GREGORIAN_CHANGE_JDN;
  const wday = ((jdn + 1) % 7 + 7) % 7;
  const leap = julian ? (((year % 4) + 4) % 4) === 0 : isLeap(year);
  const monthDays = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  let yday = d;
  for (let i = 0; i < m - 1; i++) yday += monthDays[i];
  return { year, month: m, day: d, hour, minute, second, wday, yday: yday - 1 };
}

const SHORT_DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const FULL_DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const SHORT_MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const FULL_MONTHS = ['January', 'February', 'March', 'April', 'May', 'June',
  'July', 'August', 'September', 'October', 'November', 'December'];

const pad = (n, w) => String(Math.abs(n)).padStart(w, '0');
function fmtNum(n, w) { return n < 0 ? '-' + pad(n, w) : pad(n, w); } // {:04} 带号形（-5 → -005）

function formatTimestamp(epoch, fmt, ctx) {
  // 非 -gmt 按目标时区偏移折算（%Z 出 Intl short 名、%z 出 +HHMM）。
  const tzOff = ctx.gmt ? 0 : zoneOffsetAt(epoch, ctx);
  const tzName = ctx.gmt ? 'GMT' : zoneShortName(epoch, ctx);
  const dt = epochToDatetime(epoch + tzOff);
  let out = '';
  const chars = [...fmt];
  let i = 0;
  while (i < chars.length) {
    if (chars[i] === '%' && i + 1 < chars.length) {
      i += 1;
      const c = chars[i];
      switch (c) {
        case 'Y': out += fmtNum(dt.year, 4); break;
        case 'y': out += fmtNum(dt.year % 100, 2); break;
        case 'm': out += fmtNum(dt.month, 2); break;
        case 'd': out += fmtNum(dt.day, 2); break;
        case 'e': out += String(dt.day).padStart(2, ' '); break;
        case 'H': out += fmtNum(dt.hour, 2); break;
        case 'I': {
          const h = dt.hour === 0 ? 12 : dt.hour > 12 ? dt.hour - 12 : dt.hour;
          out += fmtNum(h, 2); break;
        }
        case 'M': out += fmtNum(dt.minute, 2); break;
        case 'S': out += fmtNum(dt.second, 2); break;
        case 'p': out += dt.hour < 12 ? 'AM' : 'PM'; break;
        case 'P': out += dt.hour < 12 ? 'am' : 'pm'; break;
        case 'a': out += SHORT_DAYS[dt.wday]; break;
        case 'A': out += FULL_DAYS[dt.wday]; break;
        case 'b': case 'h': out += SHORT_MONTHS[dt.month - 1]; break;
        case 'B': out += FULL_MONTHS[dt.month - 1]; break;
        case 'j': out += fmtNum(dt.yday + 1, 3); break;
        case 'w': out += String(dt.wday); break;
        case 'Z': out += tzName; break;
        case 'z': out += renderZ(tzOff); break;
        case 'n': out += '\n'; break;
        case 't': out += '\t'; break;
        case '%': out += '%'; break;
        case 's': out += String(epoch); break;
        case 'c':
          out += `${SHORT_DAYS[dt.wday]} ${SHORT_MONTHS[dt.month - 1]} ` +
            `${String(dt.day).padStart(2, ' ')} ` +
            `${fmtNum(dt.hour, 2)}:${fmtNum(dt.minute, 2)}:${fmtNum(dt.second, 2)} ` +
            `${fmtNum(dt.year, 4)}`;
          break;
        case 'D':
          out += `${fmtNum(dt.month, 2)}/${fmtNum(dt.day, 2)}/${fmtNum(dt.year % 100, 2)}`;
          break;
        case 'T':
          out += `${fmtNum(dt.hour, 2)}:${fmtNum(dt.minute, 2)}:${fmtNum(dt.second, 2)}`;
          break;
        default:
          out += '%' + c;
      }
    } else {
      out += chars[i];
    }
    i += 1;
  }
  return out;
}

// ── clock scan -format（native scan_with_format 移植）─────────────

const isWs = (c) => /\s/.test(c);
const isDigit = (c) => c >= '0' && c <= '9';
const isAlpha = (c) => (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');

// 返回 epoch 或 null（fail = {kind:'Mismatch'|{BadZone:name}}）。
function scanWithFormat(input, fmt, base, ctx, fail) {
  const chars = [...input];
  let pos = 0;
  const f = { y: null, y2: null, mo: null, d: null, yd: null, h: null, mi: null, se: null, pm: null, epoch: null, tz: null, zoneName: null };
  while (pos < chars.length && isWs(chars[pos])) pos += 1;
  const fmtChars = [...fmt];
  let fi = 0;

  const skipWs = () => { while (pos < chars.length && isWs(chars[pos])) pos += 1; };
  const digits = (min, max) => {
    const start = pos;
    while (pos < chars.length && pos - start < max && isDigit(chars[pos])) pos += 1;
    if (pos - start < min) return null;
    const v = Number(chars.slice(start, pos).join(''));
    return Number.isFinite(v) ? v : null;
  };
  const word = () => {
    const start = pos;
    while (pos < chars.length && isAlpha(chars[pos])) pos += 1;
    if (pos === start) return null;
    return chars.slice(start, pos).join('').toLowerCase();
  };

  while (fi < fmtChars.length) {
    const c = fmtChars[fi];
    fi += 1;
    if (c === '%') {
      if (fi >= fmtChars.length) return null;
      const spec = fmtChars[fi];
      fi += 1;
      switch (spec) {
        case 'Y': skipWs(); { const v = digits(4, 4); if (v === null) return null; f.y = v; } break;
        case 'y': skipWs(); { const v = digits(1, 2); if (v === null) return null; f.y2 = v; } break;
        case 'm': skipWs(); { const v = digits(1, 2); if (v === null) return null; f.mo = v; } break;
        case 'd': case 'e': skipWs(); { const v = digits(1, 2); if (v === null) return null; f.d = v; } break;
        case 'H': case 'I': skipWs(); { const v = digits(1, 2); if (v === null) return null; f.h = v; } break;
        case 'M': skipWs(); { const v = digits(1, 2); if (v === null) return null; f.mi = v; } break;
        case 'S': skipWs(); { const v = digits(1, 2); if (v === null) return null; f.se = v; } break;
        case 'j': skipWs(); { const v = digits(1, 3); if (v === null) return null; f.yd = v; } break;
        case 'p': {
          skipWs();
          const w = word();
          if (w === null) return null;
          if (w === 'am') f.pm = false;
          else if (w === 'pm') f.pm = true;
          else return null;
          break;
        }
        case 'b': case 'h': case 'B': {
          skipWs();
          const w = word();
          if (w === null) return null;
          const mo = matchMonthName(w);
          if (mo === null) return null;
          f.mo = mo;
          break;
        }
        case 'a': case 'A': {
          skipWs();
          const w = word();
          if (w === null) return null;
          if (!WEEKDAY_NAMES.some((n) => n.startsWith(w))) return null;
          break;
        }
        case 's': {
          skipWs();
          let neg = false;
          if (chars[pos] === '-') { neg = true; pos += 1; }
          const v = digits(1, 18);
          if (v === null) return null;
          f.epoch = neg ? -v : v;
          break;
        }
        case 'z': {
          skipWs();
          const sign = chars[pos] === '+' ? 1 : chars[pos] === '-' ? -1 : 0;
          if (sign === 0) return null;
          pos += 1;
          const hh = digits(2, 2);
          if (hh === null) return null;
          if (chars[pos] === ':') pos += 1;
          const mm = digits(2, 2);
          if (mm === null) return null;
          f.tz = sign * (hh * 3600 + mm * 60);
          break;
        }
        case 'Z': {
          skipWs();
          let w = word();
          if (w === null) return null;
          // %Z token 收字母后**可选一串数字**（zp4/zp5/zp6）
          const dstart = pos;
          while (pos < chars.length && isDigit(chars[pos])) pos += 1;
          if (pos > dstart) w += chars.slice(dstart, pos).join('');
          f.zoneName = w;
          break;
        }
        case '%':
          if (chars[pos] !== '%') return null;
          pos += 1;
          break;
        case 'n':
          if (chars[pos] !== '\n') return null;
          pos += 1;
          break;
        case 't':
          if (chars[pos] !== '\t') return null;
          pos += 1;
          break;
        default:
          return null;
      }
    } else if (isWs(c)) {
      // 格式空白：输入须有至少一个空白，吞掉整段。
      if (pos >= chars.length || !isWs(chars[pos])) return null;
      skipWs();
    } else {
      if (chars[pos] !== c) return null;
      pos += 1;
    }
  }
  // 尾随空白忽略；其余尾随文本 → mismatch。
  while (pos < chars.length && isWs(chars[pos])) pos += 1;
  if (pos !== chars.length) return null;
  if (f.epoch !== null) return f.epoch;

  // -base 的历法分解（年/月/日兜底）+ 组合。
  const fieldsOf = (gmtEff) => {
    const baseCtx = gmtEff ? UTC_CTX : ctx;
    const dec = clockCivil(base, baseCtx);
    if (!dec) return null;
    const [by, bmo, bd] = dec;
    const hasY = f.y !== null ? f.y : f.y2 !== null ? mapTwoDigitYear(f.y2) : null;

    // tclsh 8.6.17 字段合成：年只在月+日（或 %j）同现生效；日可单独
    // 生效；%j 单独生效；时间只有 %H/%I 出现才整体生效。
    let yy, mo, dd;
    if (hasY !== null && f.yd !== null) {
      const t = civilFromJdn(jdnFromCivil(hasY, 1, 1) + f.yd - 1);
      [yy, mo, dd] = t;
    } else if (f.mo !== null && f.d !== null) {
      yy = hasY !== null ? hasY : by; mo = f.mo; dd = f.d;
    } else if (f.d !== null) {
      yy = by; mo = bmo; dd = f.d;
    } else if (f.yd !== null) {
      const t = civilFromJdn(jdnFromCivil(by, 1, 1) + f.yd - 1);
      yy = by; mo = t[1]; dd = t[2];
    } else {
      yy = by; mo = bmo; dd = bd;
    }

    let hour = 0, minute = 0, second = 0;
    if (f.h !== null) {
      hour = f.h;
      if (f.pm !== null) {
        const h12 = hour % 12;
        hour = f.pm ? h12 + 12 : h12;
      }
      minute = f.mi !== null ? f.mi : 0;
      second = f.se !== null ? f.se : 0;
    }
    return [yy, mo, dd, hour, minute, second];
  };

  const compute = (gmtEff) => {
    const t = fieldsOf(gmtEff);
    if (!t) return null;
    const [yy, mo, dd, hour, minute, second] = t;
    if (f.tz !== null) {
      // %z：字段按 UTC 解释后减去偏移。
      return epochFromJdn(jdnFromCivil(yy, mo, dd), hour, minute, second) - f.tz;
    }
    return clockEpoch(yy, mo, dd, hour, minute, second, gmtEff ? UTC_CTX : ctx);
  };

  // %Z：缩写表优先于 -gmt/-timezone；未知 = badTZName。
  if (f.zoneName !== null) {
    const off = zoneAbbrev(f.zoneName);
    if (off !== null) {
      const t = fieldsOf(true);
      if (!t) return null;
      const [yy, mo, dd, hour, minute, second] = t;
      return epochFromJdn(jdnFromCivil(yy, mo, dd), hour, minute, second) + off;
    }
    fail.kind = { BadZone: f.zoneName };
    return null;
  }
  return compute(ctx.gmt);
}

// ── clock scan 自由格式（native scan_free 移植）───────────────────

function freeUnit(w) {
  switch (w) {
    case 'second': case 'seconds': case 'sec': case 'secs': return 's';
    case 'minute': case 'minutes': return 'min';
    case 'hour': case 'hours': return 'hour';
    case 'day': case 'days': return 'day';
    case 'week': case 'weeks': return 'week';
    case 'fortnight': case 'fortnights': return 'fortnight';
    case 'month': case 'months': return 'month';
    case 'year': case 'years': return 'year';
    default: return null;
  }
}

function freeWeekday(w) {
  let hit = null;
  for (let i = 0; i < WEEKDAY_NAMES.length; i++) {
    if (WEEKDAY_NAMES[i].startsWith(w)) {
      if (hit !== null) return null;
      hit = i;
    }
  }
  return hit;
}

function freeMonth(w) {
  // tclsh 只收全称、三字母缩写与 "sept"。
  const ABBREV = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'];
  for (let i = 0; i < MONTH_NAMES.length; i++) {
    if (w === MONTH_NAMES[i] || w === ABBREV[i] || (i === 8 && w === 'sept')) return i + 1;
  }
  return null;
}

function meridian(h, pm) {
  if (pm) return h < 12 ? h + 12 : h;
  return h === 12 ? 0 : h;
}

// Rust {:?} 的近形转义（\ " \n \t \r；其余原样）。
function debugQuote(s) {
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

function scanFree(input, base, ctx) {
  const syntaxErr = (a, b) => clkErr(
    `unable to convert date-time string ${debugQuote(input)}: syntax error (characters ${a}-${b})`);
  const moreThanOneTime = () => clkErr(
    `unable to convert date-time string ${debugQuote(input)}: more than one time of day in string`);
  const moreThanOneZone = () => clkErr(
    `unable to convert date-time string ${debugQuote(input)}: more than one time zone in string`);

  const chars = [...input];
  // 空白分词（记字符区间）。
  const toks = [];
  let i = 0;
  while (i < chars.length) {
    if (isWs(chars[i])) { i += 1; continue; }
    const st = i;
    while (i < chars.length && !isWs(chars[i])) i += 1;
    toks.push([st, i]);
  }
  const joinTok = (t) => chars.slice(t[0], t[1]).join('');
  if (toks.length === 0) return String(base);

  const digitsOf = (cs) => {
    if (cs.length === 0 || !cs.every(isDigit)) return null;
    const v = Number(cs.join(''));
    return Number.isFinite(v) && Math.abs(v) <= Number.MAX_SAFE_INTEGER ? v : null;
  };
  const takeNum = (cs, p, min, max) => {
    const st = p[0];
    while (p[0] < cs.length && p[0] - st < max && isDigit(cs[p[0]])) p[0] += 1;
    if (p[0] - st < min) return null;
    return digitsOf(cs.slice(st, p[0]));
  };
  // ISO token：{ok, val} | {ok, null}（非 ISO 形）| {err: pos}
  function parseIsoTok(t) {
    const p = [0];
    const y = takeNum(t, p, 4, 4);
    if (y === null) return { ok: null };
    if (t[p[0]] !== '-') return { ok: null };
    p[0] += 1;
    const mo = takeNum(t, p, 1, 2);
    if (mo === null) return { ok: null };
    if (t[p[0]] !== '-') return { ok: null };
    p[0] += 1;
    const d = takeNum(t, p, 1, 2);
    if (d === null) return { ok: null };
    let [h, mi, se] = [0, 0, 0];
    let zone = null;
    if (p[0] < t.length) {
      const sep = t[p[0]];
      if (sep !== 'T' && sep !== 't' && sep !== ' ') return { err: p[0] };
      p[0] += 1;
      const hh = takeNum(t, p, 1, 2);
      if (hh === null) return { err: p[0] };
      if (t[p[0]] !== ':') return { err: p[0] };
      p[0] += 1;
      const mm = takeNum(t, p, 1, 2);
      if (mm === null) return { err: p[0] };
      h = hh; mi = mm;
      if (t[p[0]] === ':') {
        p[0] += 1;
        const ss = takeNum(t, p, 1, 2);
        if (ss === null) return { err: p[0] };
        se = ss;
      }
      if (p[0] < t.length) {
        const zc = t[p[0]];
        if (zc === 'Z' || zc === 'z') { zone = 0; p[0] += 1; }
        else if (zc === '+' || zc === '-') {
          const sign = zc === '+' ? 1 : -1;
          p[0] += 1;
          const zh = takeNum(t, p, 2, 2);
          if (zh === null) return { err: p[0] };
          const zm = takeNum(t, p, 2, 2);
          if (zm === null) return { err: p[0] };
          zone = sign * (zh * 3600 + zm * 60);
        } else return { err: p[0] };
      }
    }
    if (p[0] !== t.length) return { err: p[0] };
    return { ok: [y, mo, d, h, mi, se, zone] };
  }
  // 时刻 token：Err(pos) 为尾部垃圾位。
  function parseTimeTok(t) {
    const p = [0];
    const h0 = takeNum(t, p, 1, 2);
    if (h0 === null) return { ok: null };
    let h = h0;
    if (t[p[0]] !== ':') {
      // 无冒号：只有紧贴 am/pm 才是时刻（"3pm"）。
      if (p[0] < t.length) {
        const rest = t.slice(p[0]).join('').toLowerCase();
        if (rest === 'am') return { ok: [meridian(h, false), 0, 0] };
        if (rest === 'pm') return { ok: [meridian(h, true), 0, 0] };
      }
      return { ok: null };
    }
    p[0] += 1;
    const mi = takeNum(t, p, 1, 2);
    if (mi === null) return { err: p[0] };
    let se = 0;
    if (t[p[0]] === ':') {
      p[0] += 1;
      const x = takeNum(t, p, 1, 2);
      if (x === null) return { err: p[0] };
      se = x;
    }
    if (p[0] < t.length) {
      const rest = t.slice(p[0]).join('').toLowerCase();
      if (rest.startsWith('am')) {
        if (rest !== 'am') return { err: p[0] };
        h = meridian(h, false);
      } else if (rest.startsWith('pm')) {
        if (rest !== 'pm') return { err: p[0] };
        h = meridian(h, true);
      } else return { err: p[0] };
    }
    return { ok: [h, mi, se] };
  }

  // ── 状态 ──
  let adate = null;        // [year|null, month, day]
  let tset = null;         // [h, mi, s]
  let zone = null;         // 东向偏移秒
  let weekday = null;      // [dow, 'Plain'|'Next'|'Last']
  let ordinal = null;      // [month, isNext]
  let ordinalYear = null;
  const rel = [];          // [[n, unit], ...]
  let agoWatermark = 0;

  let ti = 0;
  while (ti < toks.length) {
    const raw = joinTok(toks[ti]);
    const lower = raw.toLowerCase();
    const core = lower.replace(/,+$/, '');
    const hadComma = core.length !== lower.length;
    const tokChars = [...raw];
    const setAdate = (a) => {
      if (ordinal !== null && !ordinal[1]) throw moreThanOneZone();
      adate = a;
    };

    // 0.5) 数值短横日期 Y-M-D（年 <100 走 %y 窗口）；两段 → token 尾 syntax error。
    if (core.includes('-')) {
      const parts = core.split('-');
      if (parts.length === 3 && parts.every((p) => p !== '' && [...p].every(isDigit))) {
        const yv = Number(parts[0]);
        const y = yv < 100 ? mapTwoDigitYear(yv) : yv;
        setAdate([y, Number(parts[1]), Number(parts[2])]);
        ti += 1;
        continue;
      }
      if (parts.length === 2 && parts.every((p) => p !== '' && [...p].every(isDigit))) {
        const e = toks[ti][1];
        throw syntaxErr(e, e);
      }
    }

    // 1) ISO 日期（带 T 的时刻形）
    {
      const r = parseIsoTok(tokChars);
      if (r.err !== undefined) {
        const g = toks[ti][0] + r.err;
        throw syntaxErr(g, g);
      }
      if (r.ok) {
        const [y, mo, d, h, mi, se, z] = r.ok;
        setAdate([y, mo, d]);
        if (h !== 0 || mi !== 0 || se !== 0 || raw.length > 10) {
          if (tset !== null) throw moreThanOneTime();
          tset = [h, mi, se];
        }
        if (z !== null) zone = z;
        ti += 1;
        continue;
      }
    }
    // 2) 8 位数字日期 YYYYMMDD
    if (core.length === 8 && [...core].every(isDigit)) {
      setAdate([Number(core.slice(0, 4)), Number(core.slice(4, 6)), Number(core.slice(6, 8))]);
      ti += 1;
      continue;
    }
    // 3) 斜杠日期 M/D[/Y[Y]]
    if (core.includes('/')) {
      const parts = core.split('/');
      if (parts.length === 2 && parts.every((p) => p !== '' && [...p].every(isDigit))) {
        setAdate([null, Number(parts[0]), Number(parts[1])]);
        ti += 1;
        continue;
      }
      if (parts.length === 3 && parts.every((p) => p !== '' && [...p].every(isDigit))) {
        const ys = parts[2];
        const y = ys.length <= 2 ? mapTwoDigitYear(Number(ys)) : Number(ys);
        setAdate([y, Number(parts[0]), Number(parts[1])]);
        ti += 1;
        continue;
      }
    }
    // 4) D-Mon-YYYY 形
    if (core.includes('-')) {
      const parts = core.split('-');
      if (parts.length === 3 && parts[0] !== '' && [...parts[0]].every(isDigit)) {
        const mo = freeMonth(parts[1]);
        if (mo !== null) {
          const d = Number(parts[0]);
          if (![...parts[2]].every(isDigit)) {
            throw syntaxErr(toks[ti][0], toks[ti][1] - 1);
          }
          const p2 = Number(parts[2]);
          const y = parts[2].length <= 2 ? mapTwoDigitYear(p2) : p2;
          setAdate([y, mo, d]);
          ti += 1;
          continue;
        }
      }
    }
    // 5) 月名优先（Mon D[, YYYY]）
    {
      const mo = freeMonth(core);
      if (mo !== null) {
        const nt = toks[ti + 1];
        if (!nt) throw syntaxErr(toks[ti][1], toks[ti][1]);
        const nraw = joinTok(nt);
        const ncore = nraw.replace(/,+$/, '');
        const ncomma = ncore.length !== nraw.length;
        const d = digitsOf([...ncore]);
        if (d === null) throw syntaxErr(toks[ti][0], toks[ti][1] - 1);
        let adv = 2;
        let year = null;
        if (ncomma) {
          const yt = toks[ti + 2];
          if (yt) {
            const ys = joinTok(yt);
            if (ys !== '' && [...ys].every(isDigit)) {
              const p2 = Number(ys);
              year = ys.length <= 2 ? mapTwoDigitYear(p2) : p2;
              adv = 3;
            }
          }
        } else if (toks[ti + 2]) {
          const ys = joinTok(toks[ti + 2]);
          // tclsh 怪形：无逗号 4 位数字——尚无时刻记 HHMM，已有则年份。
          if (ys.length === 4 && [...ys].every(isDigit)) {
            const v = Number(ys);
            if (tset !== null) year = v;
            else tset = [Math.floor(v / 100), v % 100, 0];
            adv = 3;
          }
        }
        setAdate([year, mo, d]);
        ti += adv;
        continue;
      }
    }
    // 6) 日优先：D Mon [YYYY]
    {
      const d = digitsOf([...core]);
      if (d !== null && toks[ti + 1]) {
        const nlow = joinTok(toks[ti + 1]).toLowerCase();
        const mo = freeMonth(nlow);
        if (mo !== null) {
          let adv = 2;
          let year = null;
          if (toks[ti + 2]) {
            const ys = joinTok(toks[ti + 2]);
            if (ys !== '' && [...ys].every(isDigit)) {
              const p2 = Number(ys);
              year = ys.length <= 2 ? mapTwoDigitYear(p2) : p2;
              adv = 3;
            }
          }
          setAdate([year, mo, d]);
          ti += adv;
          continue;
        }
      }
    }
    // 6.5) last <月名> 的后续数字 = 年份（%y 窗口）；数字后剩余字符
    // → syntax error 指向该字符。
    if (ordinal !== null && !ordinal[1]) {
      let cs = tokChars;
      if (cs[0] === '-') cs = cs.slice(1);
      if (cs.length > 0 && cs.every(isDigit)) {
        const ys = cs.join('');
        const v = Number(ys) || 0;
        ordinalYear = ys.length <= 2 ? mapTwoDigitYear(v) : v;
        ti += 1;
        continue;
      }
      if (cs.length > 0 && isDigit(cs[0])) {
        let ndig = 0;
        while (ndig < cs.length && isDigit(cs[ndig])) ndig += 1;
        const g = toks[ti][0] + ndig;
        const end = chars[g] === ':' ? g : Math.max(toks[ti][1] - 1, g);
        throw syntaxErr(g, end);
      }
    }
    // 7) next / last
    if (core === 'next' || core === 'last') {
      const nt = toks[ti + 1];
      if (!nt) throw syntaxErr(toks[ti][0], toks[ti][1] - 1);
      const nlow = joinTok(nt).toLowerCase();
      const dow = freeWeekday(nlow);
      if (dow !== null) { weekday = [dow, core === 'next' ? 'Next' : 'Last']; ti += 2; continue; }
      const u = freeUnit(nlow);
      if (u !== null) { rel.push([core === 'next' ? 1 : -1, u]); ti += 2; continue; }
      const mo = freeMonth(nlow);
      if (mo !== null) { ordinal = [mo, core === 'next']; ti += 2; continue; }
      throw syntaxErr(toks[ti][0], toks[ti][1] - 1);
    }
    // 7.5) 时区缩写（含军事单字母）；日期后无时刻仅 DST 缩写可接受。
    if (core !== '' && [...core].every(isAlpha)) {
      let mil = null;
      if (core.length === 1) {
        const b = core.charCodeAt(0);
        if (b >= 97 && b <= 105) mil = -((b - 97 + 1)) * 3600;            // a..i
        else if (b >= 107 && b <= 109) mil = -((b - 107 + 10)) * 3600;    // k..m
        else if (b >= 110 && b <= 121) mil = (b - 110 + 1) * 3600;        // n..y
        else if (b === 122) mil = 0;                                       // z
      }
      const off0 = mil !== null ? mil : zoneAbbrev(core);
      if (off0 !== null) {
        if (adate !== null && tset === null && !zoneIsDst(core)) {
          const e = toks[ti][1];
          throw syntaxErr(e, e);
        }
        // 日期后 DST 半时区特例（native 实测钉形）。
        let off = off0;
        if (adate !== null && zoneIsDst(core)) {
          if (core === 'ndt') off = 7200;
          else if (core === 'cadt') off = -36000;
          else if (core === 'eadt') off = -39600;
        }
        zone = -off;
        ti += 1;
        continue;
      }
    }
    // 8) 星期名
    {
      const dow = freeWeekday(core);
      if (dow !== null) { weekday = [dow, 'Plain']; ti += 1; continue; }
    }
    // 9) now / today / tomorrow / yesterday
    if (core === 'now' || core === 'today') { ti += 1; continue; }
    if (core === 'tomorrow') { rel.push([1, 'day']); ti += 1; continue; }
    if (core === 'yesterday') { rel.push([-1, 'day']); ti += 1; continue; }

    // 10) 数字时区 ±HHMM
    {
      const cs = [...core];
      if (cs.length === 5 && (cs[0] === '+' || cs[0] === '-') && cs.slice(1).every(isDigit)) {
        const sign = cs[0] === '+' ? 1 : -1;
        const zh = (cs[1].charCodeAt(0) - 48) * 10 + (cs[2].charCodeAt(0) - 48);
        const zm = (cs[3].charCodeAt(0) - 48) * 10 + (cs[4].charCodeAt(0) - 48);
        zone = sign * (zh * 3600 + zm * 60);
        ti += 1;
        continue;
      }
    }
    // 10.5) 裸单位 = +1 个单位
    {
      const u = freeUnit(core);
      if (u !== null) { rel.push([1, u]); ti += 1; continue; }
    }
    // 11) 时刻 token（其后可跟独立 am/pm）
    {
      const r = parseTimeTok(tokChars);
      if (r.err !== undefined) {
        const g = toks[ti][0] + r.err;
        throw syntaxErr(g, g);
      }
      if (r.ok) {
        let h = r.ok[0];
        const mi = r.ok[1];
        const se = r.ok[2];
        let adv = 1;
        if (toks[ti + 1]) {
          const nlow = joinTok(toks[ti + 1]).toLowerCase();
          if (nlow === 'am' || nlow === 'pm') {
            h = meridian(h, nlow === 'pm');
            adv = 2;
          }
        }
        if (tset !== null) throw moreThanOneTime();
        tset = [h, mi, se];
        ti += adv;
        continue;
      }
    }
    // 12) 数字 + 单位（相对）/ 数字 + am|pm / 裸数字（HHMM）
    {
      let cs = tokChars;
      let neg = false;
      if (cs[0] === '-') { neg = true; cs = cs.slice(1); }
      const mag = digitsOf(cs);
      const n = mag !== null ? (neg ? -mag : mag) : null;
      if (n !== null) {
        if (toks[ti + 1]) {
          const nlow = joinTok(toks[ti + 1]).toLowerCase();
          if (nlow === 'am' || nlow === 'pm') {
            if (tset !== null) throw moreThanOneTime();
            tset = [meridian(n, nlow === 'pm'), 0, 0];
            ti += 2;
            continue;
          }
          const u = freeUnit(nlow);
          if (u !== null) {
            let adv = 2;
            rel.push([n, u]);
            // "ago" 否定此前（尚未被否定过的）全部相对项。
            const agt = toks[ti + 2];
            if (agt && joinTok(agt).toLowerCase() === 'ago') {
              for (let k = agoWatermark; k < rel.length; k++) rel[k][0] = -rel[k][0];
              agoWatermark = rel.length;
              adv = 3;
            }
            ti += adv;
            continue;
          }
        }
        // 裸数字：1-2 位 = 小时，3-4 位按 HHMM。
        if (!hadComma && n >= 0 && core.length <= 4) {
          if (tset !== null) throw moreThanOneTime();
          tset = core.length <= 2 ? [n, 0, 0] : [Math.floor(n / 100), n % 100, 0];
          ti += 1;
          continue;
        }
      }
    }
    // 拼接缩写形（"est5edt"）→ more than one …。
    {
      const segs = core.split(/[0-9]/).filter((p) => p !== '');
      if (segs.length >= 2 && segs.every((p) => zoneAbbrev(p) !== null)) {
        throw tset !== null ? moreThanOneTime() : moreThanOneZone();
      }
    }
    // 无法识别：错误位细化（数字前缀后字母段 → 到 token 尾、标点 → 单字符）。
    {
      const [st0, en0] = toks[ti];
      let a = st0 > 0 ? st0 - 1 : 0;
      let b = en0 - 1;
      let q = 0;
      while (q < tokChars.length && isDigit(tokChars[q])) q += 1;
      if (q > 0 && q < tokChars.length) {
        a = toks[ti][0] + q;
        if (!isAlpha(tokChars[q])) b = a;
      }
      throw syntaxErr(a, b);
    }
  }

  // ── 组合 ──
  const baseDec = clockCivil(base, ctx);
  if (!baseDec) return String(base);
  const [by, bmo, bd, bh, bmi, bs] = baseDec;
  let [y, mo, d, h, mi, s] = [by, bmo, bd, bh, bmi, bs];

  const baseJdn = jdnFromCivil(by, bmo, bd);
  const baseWd = ((baseJdn + 1) % 7 + 7) % 7;
  if (adate !== null) {
    y = adate[0] !== null ? adate[0] : by;
    mo = adate[1];
    d = adate[2];
  } else if (weekday !== null) {
    const [dow, mode] = weekday;
    let delta;
    if (mode === 'Plain') delta = ((dow + 7 - baseWd) % 7);
    else if (mode === 'Next') delta = ((dow + 7 - baseWd) % 7) + 7;
    else {
      delta = (baseWd + 7 - dow) % 7;
      if (delta === 0) delta = 7;
      delta = -delta;
    }
    [y, mo, d] = civilFromJdn(baseJdn + delta);
  } else if (ordinal !== null) {
    const [mon, isNext] = ordinal;
    if (isNext) {
      // next <月名>：目标月 > base 月则同年，否则次年；日 = base 日
      // 夹到目标月长。
      const year = by + (mon <= bmo ? 1 : 0);
      const j0 = jdnFromCivil(year, mon, 1);
      const j1 = mon === 12 ? jdnFromCivil(year + 1, 1, 1) : jdnFromCivil(year, mon + 1, 1);
      const dim = j1 - j0;
      [y, mo, d] = civilFromJdn(j0 + Math.min(bd - 1, dim - 1));
    } else {
      // last <月名>：(年, 月, 1) 前推 2 天；显式年份同（tclsh 实测钉形）。
      const year = ordinalYear !== null ? ordinalYear : by;
      [y, mo, d] = civilFromJdn(jdnFromCivil(year, mon, 1) - 2);
    }
  }

  const dayScaleNonzero = rel.some(([n, u]) =>
    n !== 0 && !['s', 'min', 'hour'].includes(u));
  if (adate !== null || weekday !== null || ordinal !== null || dayScaleNonzero) {
    h = 0; mi = 0; s = 0;
  }
  if (tset !== null) {
    h = tset[0]; mi = tset[1]; s = tset[2];
  }

  // 相对量：日历级改字段，时间级改秒数。
  let smallSecs = 0;
  for (const [n, u] of rel) {
    if (u === 's') smallSecs += n;
    else if (u === 'min') smallSecs += n * 60;
    else if (u === 'hour') smallSecs += n * 3600;
    else if (u === 'day' || u === 'week' || u === 'fortnight') {
      const days = u === 'day' ? n : u === 'week' ? n * 7 : n * 14;
      [y, mo, d] = civilFromJdn(jdnFromCivil(y, mo, d) + days);
    } else {
      const months = u === 'year' ? n * 12 : n;
      const total = y * 12 + (mo - 1) + months;
      const ny = Math.floor(total / 12);
      const nm = ((total % 12) + 12) % 12 + 1;
      const dim = nm === 2
        ? (jdnFromCivil(ny, nm, 1) < GREGORIAN_CHANGE_JDN
          ? daysInMonthJulian(ny, 2)
          : daysInMonth(ny, 2))
        : daysInMonth(ny, nm);
      const nd = Math.min(d, dim);
      y = ny; mo = nm; d = nd;
    }
  }

  const epoch = zone !== null
    ? epochFromJdn(jdnFromCivil(y, mo, d), h, mi, s) - zone
    : clockEpoch(y, mo, d, h, mi, s, ctx);
  return String(epoch + smallSecs);
}

// ── clock add（native clock_add/apply_add 移植）───────────────────

const ADD_USAGE = 'clock add clockval ?number units?... ?-gmt boolean? ?-locale LOCALE? ?-timezone ZONE?';

function isTimeUnit(u) {
  return ['year', 'years', 'month', 'months', 'week', 'weeks', 'day', 'days',
    'hour', 'hours', 'minute', 'minutes', 'second', 'seconds'].includes(u);
}

// 按序应用各单位的调整（调用方已设时区上下文）。
function applyAdd(epoch0, pairs, ctx) {
  let epoch = epoch0;
  for (const [n, unit] of pairs) {
    if (unit === 'second' || unit === 'seconds') epoch += n;
    else if (unit === 'minute' || unit === 'minutes') epoch += n * 60;
    else if (unit === 'hour' || unit === 'hours') epoch += n * 3600;
    else if (unit === 'week' || unit === 'weeks' || unit === 'day' || unit === 'days') {
      const days = unit.startsWith('week') ? n * 7 : n;
      if (ctx.gmt) {
        epoch += days * 86400;
      } else {
        const dec = clockCivil(epoch, ctx);
        if (!dec) continue;
        const [y, mo, d, h, mi, s] = dec;
        const [ny, nm, nd] = civilFromJdn(jdnFromCivil(y, mo, d) + days);
        epoch = clockEpoch(ny, nm, nd, h, mi, s, ctx);
      }
    } else {
      // 年/月：日历月运算 + 月末夹取（儒略历区用儒略闰年规则）。
      const months = unit.startsWith('year') ? n * 12 : n;
      const dec = clockCivil(epoch, ctx);
      if (!dec) continue;
      const [y, mo, d, h, mi, s] = dec;
      const total = y * 12 + (mo - 1) + months;
      const ny = Math.floor(total / 12);
      const nm = ((total % 12) + 12) % 12 + 1;
      const calendarJdn = jdnFromCivil(ny, nm, 1);
      const dim = nm === 2
        ? (calendarJdn < GREGORIAN_CHANGE_JDN
          ? daysInMonthJulian(ny, 2)
          : daysInMonth(ny, 2))
        : daysInMonth(ny, nm);
      const nd = Math.min(d, dim);
      epoch = clockEpoch(ny, nm, nd, h, mi, s, ctx);
    }
  }
  return epoch;
}

// ── -timezone 校验（native parse_zone 同形：':' 剥除 + 组件检查 +
//    IANA 面校验——wasm 面 = Intl tzdata；native unix = zoneinfo）───

function isIanaZone(name) {
  try {
    new Intl.DateTimeFormat(DT_ANSI, { timeZone: name });
    return true;
  } catch {
    return false;
  }
}

function parseZone(raw) {
  const name = raw.startsWith(':') ? raw.slice(1) : raw;
  const parts = name.split('/');
  const ok = name !== '' && !name.startsWith('/')
    && !parts.some((c) => c === '.' || c === '..')
    && (name === 'localtime' || isIanaZone(name));
  if (!ok) {
    const disp = ':' + name;
    clkErr(`time zone "${disp}" not found`, 'CLOCK badTimeZone ' + disp);
  }
  return name === 'localtime' ? null : name;
}

// ── clock 分派（native cmd_clock 同构）───────────────────────────

const CLOCK_SUBCMDS = ['add', 'clicks', 'format', 'microseconds', 'milliseconds', 'scan', 'seconds'];
const CLOCK_SUBCMDS_LIST = 'add, clicks, format, microseconds, milliseconds, scan, or seconds';
const FORMAT_USAGE = 'clock format clockval ?-format string? ?-gmt boolean? ?-locale LOCALE? ?-timezone ZONE?';
const SCAN_USAGE = 'clock scan string ?-base seconds? ?-format string? ?-gmt boolean? ?-locale LOCALE? ?-timezone ZONE?';

const UTC_CTX = { gmt: true, zone: null };
const nowSec = () => Math.floor(Date.now() / 1000);
const nowMs = () => Date.now();

function resolveSub(word) {
  for (const s of CLOCK_SUBCMDS) if (s === word) return s;
  const hits = CLOCK_SUBCMDS.filter((s) => s.startsWith(word));
  return hits.length === 1 ? hits[0] : null; // 歧义/未知均 None
}

function clockFormat(a) {
  if (a.length < 1) clkErr(`wrong # args: should be "${FORMAT_USAGE}"`, 'CLOCK wrongNumArgs');
  const seconds = parseTclInt(a[0]);
  if (seconds === null) clkErr(`expected integer but got "${a[0]}"`, 'TCL VALUE NUMBER');
  let formatStr = '%a %b %d %H:%M:%S %Z %Y';
  let useGmt = false;
  let zone = null; // null = 未指定（本地）
  let i = 1;
  while (i < a.length) {
    const w = a[i];
    if (w === '-format' || w === '-gmt' || w === '-locale' || w === '-timezone') {
      if (i + 1 >= a.length) clkErr(`wrong # args: should be "${FORMAT_USAGE}"`, 'CLOCK wrongNumArgs');
      const val = a[i + 1];
      if (w === '-format') formatStr = val;
      else if (w === '-gmt') {
        // format 的 -gmt 是 expr 布尔（整数收，坏词 TCL VALUE NUMBER）。
        const n = parseTclInt(val);
        const b = n !== null ? n !== 0 : strictBool(val);
        if (b === null) clkErr(`expected boolean value but got "${val}"`, 'TCL VALUE NUMBER');
        useGmt = b;
      } else if (w === '-timezone') {
        zone = parseZone(val);
      }
      // -locale：收值忽略（native 同）。
      i += 2;
    } else {
      clkErr(`bad option "${w}": must be -format, -gmt, -locale, or -timezone`,
        'CLOCK badOption ' + w);
    }
  }
  if (useGmt && zone !== null) clkErr('cannot use -gmt and -timezone in same call', 'CLOCK gmtWithTimezone');
  const ctx = useGmt ? UTC_CTX : { gmt: false, zone };
  return formatTimestamp(seconds, formatStr, ctx);
}

function clockScan(a) {
  if (a.length < 1) clkErr(`wrong # args: should be "${SCAN_USAGE}"`, 'CLOCK wrongNumArgs');
  const input = a[0];
  let useGmt = false;
  let zone = null;
  let format = null;
  let base = null;
  let i = 1;
  while (i < a.length) {
    const w = a[i];
    if (w === '-base' || w === '-format' || w === '-gmt' || w === '-locale' || w === '-timezone') {
      if (i + 1 >= a.length) clkErr(`wrong # args: should be "${SCAN_USAGE}"`, 'CLOCK wrongNumArgs');
      const val = a[i + 1];
      if (w === '-gmt') {
        const b = strictBool(val);
        if (b === null) clkErr(`expected boolean value but got "${val}"`);
        useGmt = b;
      } else if (w === '-timezone') zone = parseZone(val);
      else if (w === '-format') format = val;
      else if (w === '-base') {
        const b = parseTclInt(val);
        if (b === null) clkErr(`expected integer but got "${val}"`);
        base = b;
      }
      i += 2;
    } else {
      clkErr(`bad option "${w}", must be -base, -format, -gmt, -locale or -timezone`,
        'CLOCK badOption ' + w);
    }
  }
  if (useGmt && zone !== null) clkErr('cannot use -gmt and -timezone in same call', 'CLOCK gmtWithTimezone');
  const ctx = useGmt ? UTC_CTX : { gmt: false, zone };
  if (format !== null) {
    const baseEpoch = base !== null ? base : nowSec();
    const fail = { kind: 'Mismatch' };
    const parsed = scanWithFormat(input, format, baseEpoch, ctx, fail);
    if (parsed !== null) return String(parsed);
    if (fail.kind !== 'Mismatch') {
      clkErr(`time zone "${fail.kind.BadZone}" not found`, 'CLOCK badTZName ' + fail.kind.BadZone);
    }
    clkErr('input string does not match supplied format', 'CLOCK badInputString');
  }
  const baseEpoch = base !== null ? base : nowSec();
  return scanFree(input, baseEpoch, ctx);
}

function clockAdd(a) {
  if (a.length < 1) clkErr(`wrong # args: should be "clock add clockval ?arg ...?"`);
  const clockval = parseTclInt(a[0]);
  if (clockval === null) clkErr(`expected number but got "${a[0]}"`);
  const pairs = [];
  let gmt = false;
  let zone = null;
  let i = 1;
  while (i < a.length) {
    const w = a[i];
    if (w === '-gmt') {
      if (i + 1 >= a.length) clkErr(`wrong # args: should be "${ADD_USAGE}"`);
      const val = a[i + 1];
      const b = strictBool(val);
      if (b === null) clkErr(`expected boolean value but got "${val}"`);
      gmt = b;
      i += 2;
    } else if (w === '-timezone') {
      if (i + 1 >= a.length) clkErr(`wrong # args: should be "${ADD_USAGE}"`);
      zone = parseZone(a[i + 1]);
      i += 2;
    } else if (w === '-locale') {
      if (i + 1 >= a.length) clkErr(`wrong # args: should be "${ADD_USAGE}"`);
      i += 2;
    } else {
      const count = parseTclInt(w);
      if (count === null) {
        clkErr(`bad option "${w}", must be -gmt, -locale or -timezone`, 'CLOCK badOption ' + w);
      }
      if (i + 1 >= a.length) clkErr(`wrong # args: should be "${ADD_USAGE}"`);
      const unit = a[i + 1];
      if (!isTimeUnit(unit)) {
        clkErr(`unknown unit "${unit}", must be  years, months, weeks, days, hours, minutes or seconds`,
          'CLOCK badUnit ' + unit);
      }
      pairs.push([count, unit]);
      i += 2;
    }
  }
  if (gmt && zone !== null) clkErr('cannot use -gmt and -timezone in same call', 'CLOCK gmtWithTimezone');
  const ctx = gmt ? UTC_CTX : { gmt: false, zone };
  return String(applyAdd(clockval, pairs, ctx));
}

function clockClicks(a) {
  if (a.length > 1) clkErr(`wrong # args: should be "clock clicks ?-switch?"`);
  let millis = false;
  if (a.length === 1) {
    const w = a[0];
    const isPre = (full) => full.startsWith(w);
    if (w === '-milliseconds' || (isPre('-milliseconds') && !isPre('-microseconds'))) millis = true;
    else if (w === '-microseconds' || isPre('-microseconds')) millis = false;
    else clkErr(`bad option "${w}": must be -milliseconds or -microseconds`,
      'TCL LOOKUP INDEX option ' + w);
  }
  return String(millis ? nowMs() : nowMs() * 1000);
}

function clockCmd(words) {
  if (words.length < 1) {
    clkErr('wrong # args: should be "clock subcommand ?arg ...?"', 'TCL WRONGARGS');
  }
  const sub = resolveSub(words[0]);
  const rest = words.slice(1);
  switch (sub) {
    case 'seconds':
      if (rest.length !== 0) clkErr(`wrong # args: should be "clock seconds"`);
      return String(nowSec());
    case 'clicks': return clockClicks(rest);
    case 'microseconds':
      if (rest.length !== 0) clkErr(`wrong # args: should be "clock microseconds"`);
      return String(nowMs() * 1000);
    case 'milliseconds':
      if (rest.length !== 0) clkErr(`wrong # args: should be "clock milliseconds"`);
      return String(nowMs());
    case 'format': return clockFormat(rest);
    case 'scan': return clockScan(rest);
    case 'add': return clockAdd(rest);
    default:
      clkErr(`unknown or ambiguous subcommand "${words[0]}": must be ${CLOCK_SUBCMDS_LIST}`,
        'TCL LOOKUP SUBCOMMAND ' + words[0]);
  }
}

// ── package 面（native package.rs 全实现；registry 在 JS）────────

function compareVersions(a, b) {
  const num = (s) => {
    if (!/^[+-]?[0-9]+$/.test(s)) return null;
    const v = Number(s);
    return Number.isFinite(v) && Math.abs(v) <= Number.MAX_SAFE_INTEGER ? v : null;
  };
  const pa = a.split('.').map(num).filter((v) => v !== null);
  const pb = b.split('.').map(num).filter((v) => v !== null);
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const va = i < pa.length ? pa[i] : 0;
    const vb = i < pb.length ? pb[i] : 0;
    if (va < vb) return -1;
    if (va > vb) return 1;
  }
  return 0;
}

function packageCmd(words, registry) {
  if (words.length < 1) {
    clkErr('wrong # args: should be "package package subcommand ?arg ...?"', 'TCL WRONGARGS');
  }
  const sub = words[0];
  const a = words.slice(1);
  switch (sub) {
    case 'provide': {
      if (a.length < 1 || a.length > 2) {
        clkErr('wrong # args: should be "package provide package provide name ?version?"', 'TCL WRONGARGS');
      }
      const name = a[0];
      if (a.length === 2) {
        registry.set(name, a[1]);
        return '';
      }
      // Query: 返回已提供的版本（未提供 = 空）。
      return registry.has(name) ? registry.get(name) : '';
    }
    case 'names':
      return [...registry.keys()].sort().join(' ');
    case 'forget':
      for (const name of a) registry.delete(name);
      return '';
    case 'vcompare': {
      if (a.length !== 2) {
        clkErr('wrong # args: should be "package vcompare package vcompare version1 version2"', 'TCL WRONGARGS');
      }
      return String(compareVersions(a[0], a[1]));
    }
    case 'vsatisfies': {
      if (a.length !== 2) {
        clkErr('wrong # args: should be "package vsatisfies package vsatisfies version requirement"', 'TCL WRONGARGS');
      }
      return compareVersions(a[0], a[1]) >= 0 ? '1' : '0';
    }
    case '_registry_get':
      // 内部查询面（::clockpkg::pkg_require 编排用；非公开子命令）。
      return registry.has(a[0]) ? registry.get(a[0]) : '';
    default:
      // native 无 set_error_code——errorCode 落 NONE。
      clkErr(`unknown package subcommand "${sub}": must be forget, names, provide, require, vcompare, or vsatisfies`);
  }
}

// ── 模块出口：envelope 包裹 + 注册面 ─────────────────────────────

function envelope(fn, args) {
  let result;
  try {
    result = fn(args);
  } catch (e) {
    const code = e && e.clkCode !== undefined ? e.clkCode : '';
    const msg = e && e.message !== undefined ? e.message : String(e);
    return listWords(['__CLKERR', code, msg]);
  }
  return listWords(['__CLKOK', result]);
}

export function makeClockPkgMod() {
  const registry = new Map();
  return {
    clock(...words) { return envelope(clockCmd, words); },
    package(...words) { return envelope((w) => packageCmd(w, registry), words); },
  };
}

// CLOCKPKG_SETUP_TCL — 页面 wrapper proc（顶层名补回 + 哨兵解包 +
// require 的 pkgIndex 编排）。registry 状态在 JS 模块；require 的
// auto_path/pkgIndex 流程需重入解释器（source）——native pkg_require
// 的逐步同构在 Tcl 面编排。
export const CLOCKPKG_SETUP_TCL = `
proc ::clockpkg::clock_unwrap {r} {
  if {[lindex $r 0] eq "__CLKERR"} {
    set code [lindex $r 1]
    if {$code eq ""} { set code NONE }
    return -code error -errorcode $code [lindex $r 2]
  }
  return [lindex $r 1]
}
proc ::clock {args} {
  ::clockpkg::clock_unwrap [::clockpkg::clock {*}$args]
}
proc ::clockpkg::registry_get {name} {
  set r [::clockpkg::package _registry_get $name]
  if {[lindex $r 0] eq "__CLKERR"} { return "" }
  return [lindex $r 1]
}
proc ::clockpkg::pkg_require {args} {
  if {[llength $args] < 2} {
    return -code error -errorcode {TCL WRONGARGS} {wrong # args: should be "package require package require ?-exact? name ?version?"}
  }
  set i 1
  if {[lindex $args $i] eq "-exact"} { incr i }
  if {$i >= [llength $args]} {
    return -code error -errorcode {TCL WRONGARGS} {wrong # args: should be "package require package require ?-exact? name ?version?"}
  }
  set name [lindex $args $i]
  set v [::clockpkg::registry_get $name]
  if {$v ne ""} { return $v }
  global auto_path
  if {[info exists auto_path]} {
    foreach dir $auto_path {
      set idx "$dir/pkgIndex.tcl"
      if {[file exists $idx]} {
        catch {source $idx}
        set v [::clockpkg::registry_get $name]
        if {$v ne ""} { return $v }
      }
    }
  }
  return -code error -errorcode NONE "can't find package $name"
}
proc ::package {args} {
  set sub [lindex $args 0]
  if {$sub eq "require"} {
    return [::clockpkg::pkg_require {*}$args]
  }
  ::clockpkg::clock_unwrap [::clockpkg::package {*}$args]
}
`;
