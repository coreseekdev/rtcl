//! Command implementations, split by category.

pub mod control;
pub mod loops;
pub mod string_cmds;
pub mod list;
pub mod list_sort;
pub mod dict;
pub mod array;
pub mod proc;
pub mod io;
pub mod misc;
#[cfg(feature = "regexp")]
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
