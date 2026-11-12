//! dynmod 侧模块示例 —— 一个最小的 rtcl 扩展模块（wasm32-unknown-unknown）。
//!
//! 导出面（数值 ABI；字符串/字节载荷的缓冲区交接属 v2，见
//! HANDOFF §10quater）：
//!   add(a, b)    → i64   （rtcl 侧包装为 ::myext::add，参数字符串进）
//!   version()    → i32
//!
//! 侧模块无法自注册（与宿主无共享地址空间）——命令面由宿主 JS 胶水
//! 经 RtclHandle::register_module 声明，`load myext` 挂线。
#![no_std]
use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn add(a: i64, b: i64) -> i64 {
    a + b
}

#[no_mangle]
pub extern "C" fn version() -> i32 {
    1
}
