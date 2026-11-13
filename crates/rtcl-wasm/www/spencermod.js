// spencermod.js — Spencer 全集侧模块（wasm）装载胶水 + ::regexp/::regsub
// 语义 wrapper（单一注册源：rtcl-worker.js 与 dynmod-test.html 共用，
// 各宿主 handle 只 register 一次，勿重复注册）。
//
// 结构（HANDOFF §10tredecimus）：
//   - www/spencermod.wasm = native 同源 Spencer 引擎侧模块（数值 ABI +
//     envelope 字节交接）：exec = 单次/全部匹配（跨度表），about =
//     `regexp -about` 分析面（native 输出逐字段对齐，oracle =
//     specs/regex/about-probes.tcl rtcl 侧基线）。
//   - SPENCER_SETUP_TCL = wrapper proc 全选项面（选项解析、变量写入、
//     regsub 替换、-about 按 native 输出形组装）。
//   - setupSpencerMod(handle, opts) = fetch/instantiate →
//     register_module('spencermod', glue) → load + wrapper 安装；
//     opts.wasmBytes 供 node 测试链注入字节。失败 → 返回 false
//     （调用方回退 regexpmod.js 语义子集）。

// 版本参数（页面 goto 带 ?v=）穿透 HTTP/模块缓存层——同 URL 旧缓存
// 可能钉住 wasm。
const params = new URLSearchParams(globalThis.self?.location?.search || '');
const PKG_V = params.get('v') || '';

export const SPENCER_SETUP_TCL = `
proc ::_spencer_match_vals {string spans m ng indices delta} {
  set vals {}
  set base [expr {2 * $m * $ng}]
  for {set g 0} {$g < $ng} {incr g} {
    set cs [lindex $spans [expr {$base + 2 * $g}]]
    set ce [lindex $spans [expr {$base + 2 * $g + 1}]]
    if {$cs < 0} {
      if {$indices} { lappend vals [list [expr {-1 + $delta}] [expr {-1 + $delta}]] } else { lappend vals "" }
      continue
    }
    if {$indices} {
      lappend vals [list [expr {$cs + $delta}] [expr {$ce - 1 + $delta}]]
    } else {
      if {$ce > $cs} {
        lappend vals [string range $string $cs [expr {$ce - 1}]]
      } else {
        lappend vals ""
      }
    }
  }
  return $vals
}
# 仅可从 ::regexp 调用：upvar 2 = 越过本 helper 与 ::regexp，直达
# 用户的调用帧（upvar 1 会落在 ::regexp 局部帧上，变量写丢）。
proc ::_spencer_write_vars {capVars vals ng} {
  if {[llength $capVars] == 0} { return }
  upvar 2 [lindex $capVars 0] mv
  set mv [lindex $vals 0]
  set gi 1
  foreach name [lrange $capVars 1 end] {
    upvar 2 $name sv
    if {$gi < $ng} { set sv [lindex $vals $gi] } else { set sv "" }
    incr gi
  }
}
proc ::_spencer_parse_opts {argsVar bools withStart extra} {
  upvar 1 $argsVar args
  array set o [list]
  foreach b $bools { set o($b) 0 }
  set o(start) 0
  set i 0
  set n [llength $args]
  while {$i < $n} {
    set a [lindex $args $i]
    if {[lsearch -exact $bools [string range $a 1 end]] >= 0 && [string index $a 0] eq "-"} {
      set o([string range $a 1 end]) 1
      if {$a eq "-line"} { set o(lineanchor) 1; set o(linestop) 1 }
    } elseif {$withStart && $a eq "-start"} {
      incr i
      if {$i >= $n} { error "missing argument to \\"-start\\"" }
      set o(start) [lindex $args $i]
    } elseif {$a eq "--"} {
      incr i
      break
    } elseif {[string index $a 0] eq "-"} {
      error "bad option \\"$a\\": must be -about, -all, -expanded, -indices, -inline, -line, -linestop, -lineanchor, -nocase, -start, or --"
    } else {
      break
    }
    incr i
  }
  set o(rest) [lrange $args $i end]
  return [array get o]
}
proc ::regexp {args} {
  set p [::_spencer_parse_opts args [list nocase line lineanchor linestop expanded all inline indices about] 1 0]
  array set o $p
  set rest $o(rest)
  if {[llength $rest] < 2 && !$o(about)} {
    error {wrong # args: should be "regexp ?switches? exp string ?matchVar? ?subMatchVar ...?"}
  }
  set exp [lindex $rest 0]
  set string [lindex $rest 1]
  set capVars [lrange $rest 2 end]
  if {$o(about)} {
    # native -about 输出形：numGroups + flags（specs/regex/about-probes.tcl
    # rtcl 侧基线逐字段对齐）。分析位由 spencermod about 导出提供
    # （compile_with_options + ast 分析 + 词法扫描，native cmd_regexp
    # -about 分支同构）；flags 的 0/1/multi 列表形在此组装。
    if {[llength $rest] < 1} {
      error {wrong # args: should be "regexp ?-option ...? exp string ?matchVar? ?subMatchVar ...?"}
    }
    set aflags [expr {$o(nocase) + 2 * $o(linestop) + 4 * $o(lineanchor) + 8 * $o(expanded)}]
    set r [::spencermod::about [lindex $rest 0] $aflags]
    set ng [lindex $r 0]
    set fl [lrange $r 1 end]
    set flagStr ""
    if {[llength $fl] == 1} { set flagStr [lindex $fl 0] }
    if {[llength $fl] > 1} { set flagStr $fl }
    return [list $ng $flagStr]
  }
  set flags [expr {$o(nocase) + 2 * $o(linestop) + 4 * $o(lineanchor) + 8 * $o(expanded)}]
  set r [::spencermod::exec $exp $string $flags $o(start) $o(all)]
  set nm [lindex $r 0]
  set ng [lindex $r 1]
  set spans [lrange $r 2 end]
  if {$nm == 0} { return 0 }
  # 虚拟起点（-start 越过串尾）：索引按原始 -start 计（native
  # index_delta 同）。
  set charLen [string length $string]
  set eff [expr {$o(start) < $charLen ? $o(start) : $charLen}]
  set delta [expr {$o(start) - $eff}]
  if {$o(all) && $o(inline)} {
    set out {}
    for {set m 0} {$m < $nm} {incr m} {
      foreach v [::_spencer_match_vals $string $spans $m $ng 0 $delta] { lappend out $v }
    }
    return $out
  }
  if {$o(all) && $o(indices)} {
    set out {}
    for {set m 0} {$m < $nm} {incr m} {
      foreach v [::_spencer_match_vals $string $spans $m $ng 1 $delta] { lappend out $v }
    }
    return $out
  }
  if {$o(all)} {
    # 变量取**末匹配**（native set_match_var_groups 同），返回计数
    set vals [::_spencer_match_vals $string $spans [expr {$nm - 1}] $ng 0 $delta]
    ::_spencer_write_vars $capVars $vals $ng
    return $nm
  }
  if {$o(inline)} { return [::_spencer_match_vals $string $spans 0 $ng 0 $delta] }
  if {$o(indices)} {
    set pairs [::_spencer_match_vals $string $spans 0 $ng 1 $delta]
    if {[llength $capVars] > 0} {
      ::_spencer_write_vars $capVars $pairs $ng
      return 1
    }
    return $pairs
  }
  set vals [::_spencer_match_vals $string $spans 0 $ng 0 $delta]
  ::_spencer_write_vars $capVars $vals $ng
  return 1
}
proc ::_spencer_expand {spec vals} {
  set out ""
  set i 0
  set L [string length $spec]
  set bs [format %c 92]
  while {$i < $L} {
    set c [string index $spec $i]
    if {$c eq "&"} {
      append out [lindex $vals 0]
      incr i
    } elseif {$c eq $bs && $i + 1 < $L} {
      set nx [string index $spec [expr {$i + 1}]]
      if {$nx >= "0" && $nx <= "9"} {
        append out [lindex $vals $nx]
        incr i 2
      } elseif {$nx eq $bs} {
        append out $bs
        incr i 2
      } else {
        append out $c
        incr i
      }
    } else {
      append out $c
      incr i
    }
  }
  return $out
}
proc ::regsub {args} {
  set p [::_spencer_parse_opts args [list nocase all expanded line lineanchor linestop] 1 0]
  array set o $p
  set rest $o(rest)
  if {[llength $rest] < 3} {
    error {wrong # args: should be "regsub ?switches? exp string subSpec varName"}
  }
  set exp [lindex $rest 0]
  set string [lindex $rest 1]
  set subSpec [lindex $rest 2]
  set varName [lindex $rest 3]
  set flags [expr {$o(nocase) + 2 * $o(linestop) + 4 * $o(lineanchor) + 8 * $o(expanded) + 16}]
  set r [::spencermod::exec $exp $string $flags $o(start) 1]
  set nm [lindex $r 0]
  set ng [lindex $r 1]
  set spans [lrange $r 2 end]
  set out ""
  set pos 0
  for {set m 0} {$m < $nm} {incr m} {
    set cb [lindex $spans [expr {2 * $m * $ng}]]
    set ce [lindex $spans [expr {2 * $m * $ng + 1}]]
    if {$cb < 0} { continue }
    if {$cb > $pos} { append out [string range $string $pos [expr {$cb - 1}]] }
    set vals [::_spencer_match_vals $string $spans $m $ng 0 0]
    append out [::_spencer_expand $subSpec $vals]
    set pos $ce
  }
  if {$pos < [string length $string]} { append out [string range $string $pos end] }
  if {$varName eq ""} { return $nm }
  upvar 1 $varName v
  set v $out
  return $nm
}
`;
export async function setupSpencerMod(handle, opts = {}) {
  let sp;
  const wasmUrl = 'spencermod.wasm' + (PKG_V ? '?v=' + PKG_V : '');
  try {
    if (opts.wasmBytes) {
      // node 测试链：wasm 字节直接注入（node fetch 不支持相对/文件 URL）。
      sp = (await WebAssembly.instantiate(opts.wasmBytes)).instance.exports;
    } else {
      const res = await WebAssembly.instantiateStreaming(fetch(wasmUrl));
      sp = res.instance.exports;
    }
  } catch (e) {
    const buf = await (await fetch(wasmUrl)).arrayBuffer();
    sp = (await WebAssembly.instantiate(buf)).instance.exports;
  }
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const wordsOf = (rest) =>
    // v2 dispatch（__dynmodv2:）逐词透传——rest 即原词；保留数组/
    // v1 join 形态兼容，regexpmod normalizeArgs 同约定。
    Array.isArray(rest[0])
      ? rest[0]
      : (rest.length === 1 && typeof rest[0] === 'string' && rest[0].includes('\u0001'))
        ? rest[0].split('\u0001')
        : rest;
  const glue = {
    exec(...rest) {
      const words = wordsOf(rest);
      const parts = words.map((w) => enc.encode(String(w)));
      let total = 4;
      for (const p of parts) total += 4 + p.length;
      const inBuf = new Uint8Array(total);
      const dv = new DataView(inBuf.buffer);
      dv.setUint32(0, parts.length, true);
      let o = 4;
      for (const p of parts) {
        dv.setUint32(o, p.length, true);
        inBuf.set(p, o + 4);
        o += 4 + p.length;
      }
      const ip = sp.alloc(inBuf.length);
      new Uint8Array(sp.memory.buffer, ip, inBuf.length).set(inBuf);
      sp.exec(ip, inBuf.length);
      const rp = sp.res_ptr();
      const rl = sp.res_len();
      const out = new Uint8Array(sp.memory.buffer, rp, rl);
      const odv = new DataView(sp.memory.buffer, rp, rl);
      const tag = odv.getUint32(0, true);
      if (tag !== 0) {
        let q = 4;
        const cl = odv.getUint32(q, true); q += 4;
        const code = dec.decode(new Uint8Array(sp.memory.buffer, rp + q, cl)); q += cl;
        const ml = odv.getUint32(q, true); q += 4;
        const msg = dec.decode(new Uint8Array(sp.memory.buffer, rp + q, ml));
        void code;
        throw new Error("couldn't compile regular expression pattern: " + msg);
      }
      const ngroups = odv.getUint32(4, true);
      const nmatches = odv.getUint32(8, true);
      // 平铺数字（每对两元素）——wrapper match_vals 按
      // 2*(m*ng+g) 索引；负数 = 未参与组。
      let s = nmatches + ' ' + ngroups;
      for (let i = 0; i < nmatches * ngroups; i++) {
        const b = odv.getInt32(12 + 8 * i, true);
        const e2 = odv.getInt32(16 + 8 * i, true);
        s += ' ' + b + ' ' + e2;
      }
      return s;
    },
    // -about 分析面：words = [pattern, flags]（envelope 记录表，pattern
    // 在首）→ 返回 "ngroups flag1 flag2 ..."（tclsh re_info 位序）；
    // wrapper 组装 native 输出形。编译错误 throw（exec 同文前缀）。
    about(...rest) {
      const words = wordsOf(rest);
      const parts = words.map((w) => enc.encode(String(w)));
      let total = 4;
      for (const p of parts) total += 4 + p.length;
      const inBuf = new Uint8Array(total);
      const dv = new DataView(inBuf.buffer);
      dv.setUint32(0, parts.length, true);
      let o = 4;
      for (const p of parts) {
        dv.setUint32(o, p.length, true);
        inBuf.set(p, o + 4);
        o += 4 + p.length;
      }
      const ip = sp.alloc(inBuf.length);
      new Uint8Array(sp.memory.buffer, ip, inBuf.length).set(inBuf);
      sp.about(ip, inBuf.length);
      const rp = sp.res_ptr();
      const rl = sp.res_len();
      const odv = new DataView(sp.memory.buffer, rp, rl);
      const tag = odv.getUint32(0, true);
      if (tag !== 0) {
        let q = 4;
        const cl = odv.getUint32(q, true); q += 4;
        const code = dec.decode(new Uint8Array(sp.memory.buffer, rp + q, cl)); q += cl;
        const ml = odv.getUint32(q, true); q += 4;
        const msg = dec.decode(new Uint8Array(sp.memory.buffer, rp + q, ml));
        void code;
        throw new Error("couldn't compile regular expression pattern: " + msg);
      }
      const ngroups = odv.getUint32(4, true);
      const nflags = odv.getUint32(8, true);
      let s = ngroups + '';
      let q = 12;
      for (let i = 0; i < nflags; i++) {
        const l = odv.getUint32(q, true); q += 4;
        s += ' ' + dec.decode(new Uint8Array(sp.memory.buffer, rp + q, l));
        q += l;
      }
      return s;
    },
  };
  handle.register_module('spencermod', glue);
  try {
    handle.exec('load spencermod');
    handle.exec(SPENCER_SETUP_TCL);
  } catch (e) {
    // wrapper 安装失败（Tcl 语法/引擎缺失）→ 让位给 JS 子集回退
    return false;
  }
  return !!handle.exec('namespace which -command ::regexp')
      && !!handle.exec('namespace which -command ::regsub');
}
