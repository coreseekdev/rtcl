// dynmod-bridge.js — rtcl 的 wasm `load`/`unload` 扩展模块桥。
//
// 约定（与 HANDOFF §10quater 同文档）：
//   1. JS 宿主先把侧 wasm 模块实例化好（WebAssembly.instantiate 是异步的，
//      而 Tcl 的 load 是同步命令——所以走"宿主预实例化 + 注册表挂线"）。
//   2. `rtclHandle.register_module(name, exports)` 把 exports 的函数值属性
//      声明为模块命令面；Tcl `load name` 生成 `::name::<fn>` 包装命令，
//      参数以字符串到达，返回值经 JS String() 回来。
//   3. `unload name` 解除包装；`rtclHandle.unregister_module(name)` 连内部
//      回调一起释放（实例引用由宿主丢弃 → GC）。
//
// 端到端示例（浏览器主线程或 worker 均可）：

export async function installDynmod(rtclHandle, moduleUrl) {
  // ① 实例化侧模块（wasm32-unknown-unknown 的纯 Rust crate 产物）。
  //    imports 按需给：本例不需要任何宿主导入。
  const { instance } = await WebAssembly.instantiateStreaming(
    fetch(moduleUrl), {}
  );
  const ex = instance.exports;

  // ② 侧模块的导出函数多数是数值 ABI；包装成"字符串进、字符串出"的
  //    JS 函数后交给 rtcl。数字结果 toString 即为 Tcl 值；更复杂的
  //    载荷（字符串/字节）由该胶水层负责（通过侧模块导出的
  //    rtcl_alloc/rtcl_free 做缓冲区交接——v1 先展示数值面）。
  const exportsObj = {
    add: (a, b) => ex.add(Number(a), Number(b)),
    version: () => ex.version(),
  };

  // ③ 登记：此后 Tcl 侧 `load myext` / `myext::add 20 22` / `unload myext`。
  rtclHandle.register_module("myext", exportsObj);
}

// 侧模块（Rust → wasm32-unknown-unknown）的导出面约定：
//
//   #[no_mangle] pub extern "C" fn add(a: i64, b: i64) -> i64 { a + b }
//   #[no_mangle] pub extern "C" fn version() -> i32 { 1 }
//
// cargo build --target wasm32-unknown-unknown --release 产出 myext.wasm，
// 宿主 await 实例化后交 register_module 即可。
