//! Command implementations, split by category.

pub mod control;
pub mod loops;
pub mod string_cmds;
pub mod list;
pub mod list_sort;
pub mod dict;
pub mod array;
pub mod binary;
pub mod proc;
pub mod io;
pub mod misc;
// 回溯引擎只服务全量 regexp 引擎（regexp-lite 构建没有 Full 分支）。
#[cfg(feature = "regexp")]
pub mod regexp_bt;
pub mod regexp_cmds;
#[cfg(feature = "regexp-spencer")]
// Spencer 引擎抽独立 crate：rtcl-vm（native）与 wasm 侧模块
// spencermod（见 crates/rtcl-regexp-spencer-side）共用同一实现。
pub use rtcl_regexp_spencer as regexp_spencer;
#[cfg(feature = "clock")]
pub mod clock;
#[cfg(feature = "package")]
pub mod package;
#[cfg(feature = "io")]
pub mod chan_io;
pub mod compat;
pub mod mathop;
pub mod coro;
pub mod dynmod;

/// Tcl's `can not find channel named "x"` failure (feature-free home:
/// event.rs needs it without the io feature; chan_io re-exports).
pub(crate) fn chan_not_found(interp: &mut super::Interp, id: &str) -> crate::error::Error {
    list::set_error_code(
        interp,
        &format!("TCL LOOKUP CHANNEL {}", crate::value::tcl_quote(id)),
    );
    crate::error::Error::runtime(
        format!("can not find channel named \"{}\"", id),
        crate::error::ErrorCode::Io,
    )
}
#[cfg(feature = "exec")]
pub mod exec_cmd;
pub mod namespace;
#[cfg(any(feature = "file", feature = "signal", feature = "exec"))]
pub mod os;
pub mod introspect;
#[cfg(feature = "json")]
pub mod json;
pub mod trace;
pub mod words;
#[cfg(feature = "std")]
pub mod event;
#[cfg(feature = "std")]
pub mod interp_cmd;
pub mod oo;
