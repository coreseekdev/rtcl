// Node test for rtcl-wasm. Run via build.sh first, then: node test.js
// Exit code 0 = all assertions passed.
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
        if (detail !== undefined) {
            console.error(`      ${detail}`);
        }
    }
}

function assertEq(actual, expected, label) {
    ok(actual === expected, label,
        `expected: ${JSON.stringify(expected)}, actual: ${JSON.stringify(actual)}`);
}

function assertThrows(fn, substr, label) {
    try {
        fn();
        ok(false, label, `expected throw containing ${JSON.stringify(substr)}, nothing thrown`);
    } catch (e) {
        const msg = String(e && e.message || e);
        ok(msg.includes(substr), label, `expected throw containing ${JSON.stringify(substr)}, got: ${msg}`);
    }
}

async function test() {
    const { default: init, RtclHandle } = await import("./pkg/rtcl_wasm.js");
    await init({ module_or_path: readFileSync(join(__dirname, "pkg/rtcl_wasm_bg.wasm")) });

    // ── Regression: core exec ────────────────────────────────────────────
    const rtcl = new RtclHandle();
    assertEq(rtcl.exec("expr {1 + 2}"), "3", "exec: expr arithmetic");
    assertEq(rtcl.exec("set x 42; expr {$x * 2}"), "84", "exec: variable state persists");
    assertEq(rtcl.exec("proc add2 {a b} { expr {$a + $b} }; add2 3 4"), "7", "exec: user proc");
    assertEq(rtcl.exec("string length \"Hello World\""), "11", "exec: string length");
    assertEq(rtcl.exec("set lst {1 2 3}; lappend lst 4; set lst"), "1 2 3 4", "exec: lappend");
    assertEq(rtcl.exec("set sum 0; for {set i 1} {$i <= 10} {incr i} { incr sum $i }; set sum"), "55", "exec: for loop");
    assertThrows(() => rtcl.exec("expr {1 / 0}"), "divide by zero", "exec: error propagates to JS");

    // ── JS-registered commands ───────────────────────────────────────────
    rtcl.register_command("greet", (name) => `Hello ${name} from JS!`);
    assertEq(rtcl.exec("greet World"), "Hello World from JS!", "js command: single arg");

    rtcl.register_command("add", (a, b) => Number(a) + Number(b));
    assertEq(rtcl.exec("add 2 3"), "5", "js command: multiple args, numeric return");

    rtcl.register_command("pick", (...args) => args.join("|"));
    assertEq(rtcl.exec("pick a b c"), "a|b|c", "js command: rest args");

    // ── Per-instance isolation ───────────────────────────────────────────
    const a = new RtclHandle();
    const b = new RtclHandle();
    a.register_command("helper", (x) => `A:${x}`);
    b.register_command("helper", (x) => `B:${x}`);
    assertEq(a.exec("helper q"), "A:q", "isolation: instance A uses its own callback");
    assertEq(b.exec("helper q"), "B:q", "isolation: instance B uses its own callback");

    // ── Unregister ───────────────────────────────────────────────────────
    rtcl.register_command("temp_cmd", () => "temp");
    assertEq(rtcl.exec("temp_cmd"), "temp", "unregister: command works before removal");
    assertEq(rtcl.unregister_command("temp_cmd"), true, "unregister: returns true for existing command");
    assertThrows(() => rtcl.exec("temp_cmd"), "invalid command", "unregister: command gone after removal");
    assertEq(rtcl.unregister_command("temp_cmd"), false, "unregister: returns false for missing command");

    // ── Output handler (puts redirection) ────────────────────────────────
    const out = [];
    rtcl.set_output_handler((s) => out.push(s));
    rtcl.exec('puts "hello"; puts -nonewline "no-newline"');
    assertEq(out.join(""), "hello\nno-newline", "output handler: puts writes routed to JS");

    // ── JS error propagation (readable message, not Rust Debug noise) ────
    rtcl.register_command("boom", () => { throw new Error("kaboom"); });
    let boomMsg = "";
    try { rtcl.exec("boom"); } catch (e) { boomMsg = String(e && e.message || e); }
    assertEq(boomMsg, "JS command 'boom' error: kaboom", "js error: message propagates cleanly");

    console.log(`\n${passed} passed, ${failed} failed`);
    if (failed > 0) process.exit(1);
}

test().catch((e) => {
    console.error(e);
    process.exit(1);
});
