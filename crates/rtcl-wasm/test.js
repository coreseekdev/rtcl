const { RtclHandle } = require("./pkg/rtcl_wasm.js");

async function test() {
    const rtcl = new RtclHandle();
    console.log("=== rtcl-wasm test ===\n");

    // 1. 基本表达式
    let r = rtcl.exec("expr {1 + 2}");
    console.log("1 + 2 =", r);

    // 2. 变量
    r = rtcl.exec("set x 42; expr {$x * 2}");
    console.log("42 * 2 =", r);

    // 3. proc
    r = rtcl.exec("proc add {a b} { expr {$a + $b} }; add 3 4");
    console.log("add 3 4 =", r);

    // 4. 字符串操作
    r = rtcl.exec("string length \"Hello World\"");
    console.log("string length 'Hello World' =", r);

    // 5. 列表操作
    r = rtcl.exec("set lst {1 2 3}; lappend lst 4; set lst");
    console.log("lappend {1 2 3} 4 =", r);

    // 6. 条件
    r = rtcl.exec("if {1 > 0} { set result yes } else { set result no }; set result");
    console.log("1 > 0 =", r);

    // 7. 循环
    r = rtcl.exec("set sum 0; for {set i 1} {$i <= 10} {incr i} { incr sum $i }; set sum");
    console.log("sum 1..10 =", r);

    // 8. JS 回调
    rtcl.register_command("__greet", (name) => {
        return "Hello " + name + " from JS!";
    });
    r = rtcl.exec("__greet World");
    console.log("__greet World =", r);

    // 9. 错误处理
    try {
        rtcl.exec("expr {1 / 0}");
    } catch (e) {
        console.log("1 / 0 error:", e.message || e);
    }

    console.log("\n=== All tests passed ===");
}

test().catch(console.error);
