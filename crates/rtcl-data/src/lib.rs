//! rtcl-data — rtcl 宿主可复用的通用无状态数据原语。
//!
//! 家族：`url`（URL 规范化 + 确定性 key，zk 血统）/ `sha`（十六进制摘要）/
//! `yaml`（serde_yaml ↔ rtcl Value 桥，镜像 core json.rs 约定）/
//! `fs`（原子写）/ `register`（命令批量注册）。
//!
//! 纯 std crate：不进 rtcl-wasm 构图（同 rtcl-expect 先例）。
//! 宿主两条用法：直接调 Rust 函数；或 `rtcl_data::register(&mut interp)` 挂命令。

pub mod error;
pub mod fs;
pub mod register;
pub mod sha;
pub mod url;
pub mod yaml;

pub use error::{DataError, Result};
