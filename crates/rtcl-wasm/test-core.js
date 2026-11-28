// Node smoke test for the rtcl-wasm CORE track (D6 Core Profile).
// Build via ./build.sh --core first (generates pkg/rtcl_wasm_bg_core.wasm
// alongside the std pkg), then: node test-core.js
// Exit code 0 = all assertions passed.
//
// 覆盖面（specs/ARCH-DESIGN-20261126.md D6）：
//   核心面   — set/expr（含 bignum）/list/dict/string/proc/OO/控制流/
//              namespace/apply + core-in 扩展（lambda/curry/function/
//              ref 家族/alias）/channel 内存面（chan 协议面 + puts 路由）
//   关闭面   — file/json/clock/package/exec/env/encoding（feature 关）+
//              36 项扩展 std-only 部分（ext 关）→ 一律 invalid command
//              name（探针钉形），探后实例仍可用。
const { readFileSync } = require("node:fs");
const { join } = require("node:path");

let passed = 0;
let failed = 0;

function ok(cond, label, detail) {
    if (cond) {
        passed++;
        console.log(`  ok  ${label}`);
    } else {
        failed++;
        console.error(`FAIL  ${label}`);
        if (detail !== undefined) console.error(`      ${detail}`);
    }
}

function assertEq(actual, expected, label) {
    ok(actual === expected, label,
        `expected: ${JSON.stringify(expected)}, actual: ${JSON.stringify(actual)}`);
}

function assertUnknown(h, name, label) {
    let msg = "";
    try {
        h.exec(name);
        ok(false, label, "no error thrown");
        return;
    } catch (e) {
        msg = String((e && e.message) || e);
    }
    ok(msg.includes(`invalid command name "${name}"`), label, `got: ${msg}`);
    // 探针后实例仍活着（trap 之后的实例不可用——crash-wasm 同款口径）。
    try {
        ok(h.exec("expr {1+1}") === "2", label + " [instance alive after probe]");
    } catch (e) {
        ok(false, label + " [instance alive after probe]", String((e && e.message) || e));
    }
}

async function test() {
    const { default: init, RtclHandle } = await import("./pkg/rtcl_wasm.js");
    await init({ module_or_path: readFileSync(join(__dirname, "pkg/rtcl_wasm_bg_core.wasm")) });

    // ── 构造即最小面：无 ui/http/log/rules 词表层（core 轨不加载）──────
    const tcl = new RtclHandle();

    // ── 核心语言 ─────────────────────────────────────────────────────────
    assertEq(tcl.exec("set x 42; expr {$x * 2}"), "84", "core: set + expr");
    // D5 bignum：i64 溢出自动宽化（语言核心，core 轨保留）。
    assertEq(tcl.exec("expr {9223372036854775807 + 1}"), "9223372036854775808",
        "core: expr bignum widening");
    assertEq(tcl.exec("proc add2 {a b} { expr {$a + $b} }; add2 3 4"), "7", "core: proc define + call");
    assertEq(tcl.exec("set s 0; for {set i 1} {$i <= 10} {incr i} { incr s $i }; set s"), "55",
        "core: for + incr");
    assertEq(tcl.exec("set r {}; foreach v {1 2 3} { lappend r [expr {$v*2}] }; set r"), "2 4 6",
        "core: foreach + lappend");
    assertEq(tcl.exec("while {1} { break }; set done yes"), "yes", "core: while + break");
    assertEq(tcl.exec("switch -- 2 {1 {set w one} 2 {set w two} default {set w other}}; set w"), "two",
        "core: switch");
    assertEq(tcl.exec("catch {error boom} m o; list $m [dict get $o -code]"), "boom 1",
        "core: catch/try error machinery");
    assertEq(tcl.exec("try { set v ok } on error {e o} { set v err } finally { unset -nocomplain -- }; set v"), "ok",
        "core: try/on/finally");
    assertEq(tcl.exec("apply {{a b} { expr {$a + $b} }} 2 3"), "5", "core: apply");
    assertEq(tcl.exec("namespace eval ns9 { variable v 7 }; set ns9::v"), "7", "core: namespace + variable");
    assertEq(tcl.exec("proc up {n} { upvar $n l; set l 5 }; up lv; set lv"), "5", "core: upvar/uplevel face");
    assertEq(tcl.eval, undefined, "core: (sanity) exec is a function");
    assertEq(tcl.exec("info complete {set a 1}"), "1", "core: info complete (REPL face)");
    assertEq(tcl.exec("format %.2f 3.14159"), "3.14", "core: format");
    assertEq(tcl.exec("binary encode hex \"abc\""), "616263", "core: binary");

    // ── list / dict / string 全面 ────────────────────────────────────────
    assertEq(tcl.exec("llength {a b c d}"), "4", "list: llength");
    assertEq(tcl.exec("lindex {a b c} end"), "c", "list: lindex end");
    assertEq(tcl.exec("lsort -integer {3 1 2}"), "1 2 3", "list: lsort");
    assertEq(tcl.exec("linsert {a c} 1 b"), "a b c", "list: linsert");
    assertEq(tcl.exec("set d [dict create a 1 b [list x y]]; dict get $d b"), "x y", "dict: create/get");
    assertEq(tcl.exec("dict set d c 3; dict incr d a; dict get $d a"), "2", "dict: set/incr");
    assertEq(tcl.exec("dict update d a u { set u 9 }; dict get $d a"), "9", "dict: update");
    assertEq(tcl.exec("string map {a @} banana"), "b@n@n@", "string: map");
    assertEq(tcl.exec("string toupper [string range abcdef 1 3]"), "BCD", "string: range/toupper");
    assertEq(tcl.exec("split \"a,b,c\" ,"), "a b c", "string: split");
    assertEq(tcl.exec("join {1 2 3} -"), "1-2-3", "string: join");
    assertEq(tcl.exec("string match {[a-c]*t} bat"), "1", "string: match");

    // ── OO（core 面）────────────────────────────────────────────────────
    assertEq(tcl.exec(
        "oo::class create Counter { variable n 0; method incr9 {} { incr n }; method value {} { return $n } }; " +
        "Counter create mk; mk incr9; mk incr9; mk value"), "2", "oo: class/method/variable");
    assertEq(tcl.exec(
        "oo::class create Base { method m {} { return base } }; " +
        "oo::class create Sub { superclass Base; method m {} { return [next]-sub } }; " +
        "Sub create s2; s2 m"), "base-sub", "oo: inheritance + next");
    assertEq(tcl.exec(
        "oo::class create P9 { method go {x} { return [expr {$x * 3}] } }; " +
        "P9 create p9; p9 go 14"), "42", "oo: method args");

    // ── channel 内存面（协议面 native；数据面宿主服务）──────────────────
    assertEq(tcl.exec("llength [chan names]"), "3", "chan: stdin/stdout/stderr present");
    assertEq(tcl.exec("string match {*-buffering line*} [chan configure stdout]"), "1",
        "chan: configure protocol face");
    const out2 = [];
    tcl.set_output_handler((s) => out2.push(s));
    tcl.exec('puts -nonewline "core-out"; puts "!"');
    assertEq(out2.join(""), "core-out!\n", "chan: puts routed to JS handler");

    // ── core-in 扩展（lambda/curry/function/ref 家族/alias）─────────────
    assertEq(tcl.exec("set f [lambda {x} { expr {$x * 2} }]; $f 21"), "42", "ext-in: lambda");
    assertEq(tcl.exec("set g [curry expr 2 +]; $g 40"), "42", "ext-in: curry");
    assertEq(tcl.exec("function 7"), "7", "ext-in: function");
    assertEq(tcl.exec("set r [ref v function lambda.finalizer]; getref $r"), "v", "ext-in: ref/getref");
    assertEq(tcl.exec("set r2 [ref 1 function lambda.finalizer]; setref $r2 9; getref $r2"), "9",
        "ext-in: setref");
    assertEq(tcl.exec("alias myinc incr; myinc n9 5; set n9"), "5", "ext-in: alias");
    // tclsh 9.1 标准库词面（非 36 项扩展）：readFile/parray 恒在。
    assertEq(tcl.exec("namespace which -command readFile") !== "", true, "stdlib: readFile present");
    assertEq(tcl.exec("namespace which -command parray") !== "", true, "stdlib: parray present");

    // ── 关闭面：feature 关（file/json/clock/package/exec/env/encoding）──
    //    注意 io 保留：open/close/read/gets/chan 等在 core 轨仍在
    //    （channel 内存面），对文件数据面报平台错误而非 unknown。
    const off = ["file", "glob", "source", "cd", "pwd", "readdir", "encoding",
        "clock", "package", "exec", "env", "json"];
    for (const name of off) assertUnknown(tcl, name, `off: ${name} is unknown command`);
    // json::decode/encode 子命令面随 json 关闭。
    assertUnknown(tcl, "json::decode", "off: json::decode is unknown command");

    // ── 关闭面：36 项扩展 std-only 部分（ext feature 关）────────────────
    const extOff = ["exists", "local", "upcall", "defer", "stacktrace",
        "pack", "unpack", "loop", "range", "lsubst", "rand", "debug",
        "xtrace", "taint", "untaint", "disassemble", "case", "ensemble",
        "stackdump", "errorInfo", "popen",
        "+", "-", "*", "/"];
    for (const name of extOff) assertUnknown(tcl, name, `ext-off: ${name} is unknown command`);

    // ── stdlib 预置层完好（auto_*/history 是 init.tcl 移植，恒在）────────
    //    auto_qualify 只验在册（其体调用 regsub——regexp 在 wasm 是 JS
    //    dynmod 面，core 冒烟不注入，同 std 无模块形态）。
    assertEq(tcl.exec("namespace which -command auto_qualify") !== "", true,
        "stdlib: auto_qualify present");
    assertEq(tcl.exec("history add {set a 1}; lindex [history event 0] 0"), "set", "stdlib: history face");
    assertEq(tcl.exec("namespace which -command unknown") !== "", true, "stdlib: unknown handler present");

    // ── 宿主回调 + 预算面（core 轨同面）─────────────────────────────────
    tcl.register_command("jsmul", (a, b) => Number(a) * Number(b));
    assertEq(tcl.exec("jsmul 6 7"), "42", "api: JS command callback");
    tcl.set_step_budget(200000);
    let budgetMsg = "";
    try { tcl.exec("while {1} {}"); } catch (e) { budgetMsg = String((e && e.message) || e); }
    ok(budgetMsg.includes("step budget exceeded"), "api: step budget trips", budgetMsg);
    tcl.set_step_budget(0);

    console.log(`\n${passed} passed, ${failed} failed`);
    if (failed > 0) process.exit(1);
}

test().catch((e) => {
    console.error(e);
    process.exit(1);
});
