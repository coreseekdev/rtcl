//! Command registry — built-in registration and CmdId dispatch.

use super::Interp;
use crate::command::{CommandFunc, CommandCategory};
use crate::error::{Error, Result};
use rtcl_parser::CmdId;

impl Interp {
    /// Register all built-in commands.
    pub(super) fn register_builtins(&mut self) {
        use super::commands::*;
        use CommandCategory::*;

        // -- Language builtins: core Tcl language primitives ------------------
        self.register_categorized("set", misc::cmd_set, Language);
        self.register_categorized("if", control::cmd_if, Language);
        self.register_categorized("while", loops::cmd_while, Language);
        self.register_categorized("for", loops::cmd_for, Language);
        self.register_categorized("foreach", loops::cmd_foreach, Language);
        self.register_categorized("switch", control::cmd_switch, Language);
        self.register_categorized("break", control::cmd_break, Language);
        self.register_categorized("continue", control::cmd_continue, Language);
        self.register_categorized("return", control::cmd_return, Language);
        self.register_categorized("exit", control::cmd_exit, Language);
        self.register_categorized("proc", proc::cmd_proc, Language);
        self.register_categorized("rename", proc::cmd_rename, Language);
        self.register_categorized("eval", proc::cmd_eval, Language);
        self.register_categorized("apply", proc::cmd_apply, Language);
        self.register_categorized("uplevel", proc::cmd_uplevel, Language);
        self.register_categorized("upvar", proc::cmd_upvar, Language);
        self.register_categorized("global", proc::cmd_global, Language);
        self.register_categorized("unset", misc::cmd_unset, Language);
        self.register_categorized("expr", misc::cmd_expr, Language);
        self.register_categorized("catch", control::cmd_catch, Language);
        self.register_categorized("error", control::cmd_error, Language);
        self.register_categorized("try", control::cmd_try, Language);
        self.register_categorized("tailcall", control::cmd_tailcall, Language);
        self.register_categorized("subst", misc::cmd_subst, Language);
        self.register_categorized("incr", misc::cmd_incr, Language);
        self.register_categorized("append", misc::cmd_append, Language);
        self.register_categorized("info", misc::cmd_info, Language);

        // -- Standard library: data manipulation commands ---------------------
        self.register_categorized("string", string_cmds::cmd_string, Standard);
        self.register_categorized("list", list::cmd_list, Standard);
        self.register_categorized("llength", list::cmd_llength, Standard);
        self.register_categorized("lindex", list::cmd_lindex, Standard);
        self.register_categorized("lappend", list::cmd_lappend, Standard);
        self.register_categorized("lrange", list::cmd_lrange, Standard);
        self.register_categorized("lsearch", list::cmd_lsearch, Standard);
        self.register_categorized("lsort", list::cmd_lsort, Standard);
        self.register_categorized("linsert", list::cmd_linsert, Standard);
        self.register_categorized("lreplace", list::cmd_lreplace, Standard);
        self.register_categorized("lassign", list::cmd_lassign, Standard);
        self.register_categorized("lrepeat", list::cmd_lrepeat, Standard);
        self.register_categorized("lreverse", list::cmd_lreverse, Standard);
        self.register_categorized("concat", list::cmd_concat, Standard);
        self.register_categorized("split", list::cmd_split, Standard);
        self.register_categorized("join", list::cmd_join, Standard);
        self.register_categorized("lmap", list::cmd_lmap, Standard);
        self.register_categorized("lset", list::cmd_lset, Standard);
        self.register_categorized("dict", dict::cmd_dict, Standard);
        self.register_categorized("array", array::cmd_array, Standard);
        self.register_categorized("format", io::cmd_format, Standard);
        self.register_categorized("scan", misc::cmd_scan, Standard);
        self.register_categorized("range", loops::cmd_range, Standard);
        self.register_categorized("time", loops::cmd_time, Standard);
        self.register_categorized("timerate", loops::cmd_timerate, Standard);

        // -- Extension: platform / optional commands -------------------------
        self.register_categorized("puts", io::cmd_puts, Extension);
        self.register_categorized("disassemble", misc::cmd_disassemble, Extension);

        #[cfg(feature = "std")]
        {
            self.register_categorized("source", io::cmd_source, Extension);
            self.register_categorized("file", io::cmd_file, Extension);
            self.register_categorized("glob", io::cmd_glob, Extension);
            self.register_categorized("regexp", regexp_cmds::cmd_regexp, Extension);
            self.register_categorized("regsub", regexp_cmds::cmd_regsub, Extension);
        }
    }

    // -- Command registration ------------------------------------------------

    fn register_categorized(&mut self, name: &str, func: CommandFunc, cat: CommandCategory) {
        self.commands.insert(name.to_string(), func);
        self.command_categories.insert(name.to_string(), cat);
    }

    /// Register an external command (always categorised as Extension).
    pub fn register_command(&mut self, name: &str, func: CommandFunc) {
        self.register_categorized(name, func, CommandCategory::Extension);
    }

    pub fn delete_command(&mut self, name: &str) -> Result<()> {
        if self.commands.remove(name).is_none() {
            return Err(Error::invalid_command(name));
        }
        self.command_categories.remove(name);
        Ok(())
    }

    pub fn command_exists(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    // -- Call dispatch (by numeric CmdId) ------------------------------------

    /// Map a `CmdId` (u16) to the corresponding command function.
    pub(crate) fn resolve_cmd(&self, cmd_id: u16) -> Option<CommandFunc> {
        use super::commands::*;
        Some(match cmd_id {
            // -- Language commands (standard) --
            x if x == CmdId::Foreach   as u16 => loops::cmd_foreach,
            x if x == CmdId::Switch    as u16 => control::cmd_switch,
            x if x == CmdId::Try       as u16 => control::cmd_try,
            x if x == CmdId::Catch     as u16 => control::cmd_catch,
            x if x == CmdId::Proc      as u16 => proc::cmd_proc,
            x if x == CmdId::Rename    as u16 => proc::cmd_rename,
            x if x == CmdId::Eval      as u16 => proc::cmd_eval,
            x if x == CmdId::Apply     as u16 => proc::cmd_apply,
            x if x == CmdId::Uplevel   as u16 => proc::cmd_uplevel,
            x if x == CmdId::Upvar     as u16 => proc::cmd_upvar,
            x if x == CmdId::Global    as u16 => proc::cmd_global,
            x if x == CmdId::Unset     as u16 => misc::cmd_unset,
            x if x == CmdId::Subst     as u16 => misc::cmd_subst,
            x if x == CmdId::Info      as u16 => misc::cmd_info,
            x if x == CmdId::Error     as u16 => control::cmd_error,
            x if x == CmdId::Tailcall  as u16 => control::cmd_tailcall,
            x if x == CmdId::Append    as u16 => misc::cmd_append,
            x if x == CmdId::StringCmd as u16 => string_cmds::cmd_string,
            x if x == CmdId::List      as u16 => list::cmd_list,
            x if x == CmdId::Llength   as u16 => list::cmd_llength,
            x if x == CmdId::Lindex    as u16 => list::cmd_lindex,
            x if x == CmdId::Lappend   as u16 => list::cmd_lappend,
            x if x == CmdId::Lrange    as u16 => list::cmd_lrange,
            x if x == CmdId::Lsearch   as u16 => list::cmd_lsearch,
            x if x == CmdId::Lsort     as u16 => list::cmd_lsort,
            x if x == CmdId::Linsert   as u16 => list::cmd_linsert,
            x if x == CmdId::Lreplace  as u16 => list::cmd_lreplace,
            x if x == CmdId::Lassign   as u16 => list::cmd_lassign,
            x if x == CmdId::Lrepeat   as u16 => list::cmd_lrepeat,
            x if x == CmdId::Lreverse  as u16 => list::cmd_lreverse,
            x if x == CmdId::Concat    as u16 => list::cmd_concat,
            x if x == CmdId::Split     as u16 => list::cmd_split,
            x if x == CmdId::Join      as u16 => list::cmd_join,
            x if x == CmdId::Lmap      as u16 => list::cmd_lmap,
            x if x == CmdId::Lset      as u16 => list::cmd_lset,
            x if x == CmdId::Dict      as u16 => dict::cmd_dict,
            x if x == CmdId::Array     as u16 => array::cmd_array,
            x if x == CmdId::Format    as u16 => io::cmd_format,
            x if x == CmdId::Scan      as u16 => misc::cmd_scan,
            x if x == CmdId::Range     as u16 => loops::cmd_range,
            x if x == CmdId::Time      as u16 => loops::cmd_time,
            x if x == CmdId::Timerate  as u16 => loops::cmd_timerate,
            // -- Extension commands --
            x if x == CmdId::Puts        as u16 => io::cmd_puts,
            x if x == CmdId::Disassemble as u16 => misc::cmd_disassemble,
            #[cfg(feature = "std")]
            x if x == CmdId::Source as u16 => io::cmd_source,
            #[cfg(feature = "std")]
            x if x == CmdId::File   as u16 => io::cmd_file,
            #[cfg(feature = "std")]
            x if x == CmdId::Glob   as u16 => io::cmd_glob,
            #[cfg(feature = "std")]
            x if x == CmdId::Regexp as u16 => regexp_cmds::cmd_regexp,
            #[cfg(feature = "std")]
            x if x == CmdId::Regsub as u16 => regexp_cmds::cmd_regsub,
            _ => return None,
        })
    }

    /// Return the category for a command, if it exists.
    pub fn command_category(&self, name: &str) -> Option<CommandCategory> {
        self.command_categories.get(name).copied()
    }
}
