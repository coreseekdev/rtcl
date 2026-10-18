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
#[cfg(any(feature = "regexp", feature = "regexp-lite"))]
pub mod regexp_bt;
pub mod regexp_cmds;
#[cfg(feature = "clock")]
pub mod clock;
#[cfg(feature = "package")]
pub mod package;
#[cfg(feature = "io")]
pub mod chan_io;
#[cfg(feature = "exec")]
pub mod exec_cmd;
pub mod namespace;
#[cfg(any(feature = "file", feature = "signal", feature = "exec"))]
pub mod os;
pub mod introspect;
pub mod json;
pub mod trace;
pub mod words;
#[cfg(feature = "std")]
pub mod event;
#[cfg(feature = "std")]
pub mod interp_cmd;
pub mod oo;
