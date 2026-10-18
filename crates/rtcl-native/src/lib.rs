//! rtcl-native —— rtcl 的原生宿主服务实现（std 后端）。
//!
//! 与 rtcl-wasm 对偶：wasm 侧把宿主 trait 接到 JS（JsConsole/JsClock/JS
//! fetch 桥）；native 侧把宿主 trait 接到 std / reqwest，并注册对应桥命令。
//!
//! 现有模块：
//! - `net`（feature `net`）：[`rtcl_core::host::HostHttp`] 的 reqwest
//!   blocking 实现 + `__rtcl_http` / `__rtcl_http_cancel` 桥命令。
//!
//! 路线（`rtcl-core/src/host.rs` 的「逐个收敛」）：fs / env / exec / clock
//! 的 std 实现后续收进本 crate，各自 feature 门控，保持 wasm / no-std
//! 组合不携带多余能力。

#[cfg(feature = "net")]
pub mod net;
