//! Variable access methods on [`Interp`].
//!
//! All variable reads/writes are scope-aware: at global level they use
//! `Interp::globals`; inside a proc they use the current `CallFrame`'s
//! locals (or follow upvar links to globals / other frames).

use super::util::split_array_ref;
use super::{Interp, UpvarLink};
use crate::error::{Error, Result};
use crate::value::Value;

impl Interp {
    // ── internal helpers ────────────────────────────────────────

    /// Resolve a variable name, following upvar links.
    /// Returns `Some(&Value)` if found.
    fn resolve_var(&self, name: &str) -> Option<&Value> {
        if let Some(frame) = self.frames.last() {
            if let Some(link) = frame.upvars.get(name) {
                return match link {
                    UpvarLink::Global(gname) => self.globals.get(gname.as_str()),
                    UpvarLink::Frame { frame_index, var_name } => {
                        self.frames.get(*frame_index)
                            .and_then(|f| f.locals.get(var_name.as_str()))
                    }
                };
            }
            frame.locals.get(name)
        } else {
            self.globals.get(name)
        }
    }

    /// Set a variable in the current scope, following upvar links.
    fn store_var(&mut self, name: &str, value: Value) {
        if self.frames.is_empty() {
            self.globals.insert(name.to_string(), value);
            return;
        }
        let frame_idx = self.frames.len() - 1;
        if let Some(link) = self.frames[frame_idx].upvars.get(name).cloned() {
            match link {
                UpvarLink::Global(gname) => {
                    self.globals.insert(gname, value);
                }
                UpvarLink::Frame { frame_index, var_name } => {
                    if let Some(f) = self.frames.get_mut(frame_index) {
                        f.locals.insert(var_name, value);
                    }
                }
            }
        } else {
            self.frames[frame_idx].locals.insert(name.to_string(), value);
        }
    }

    /// Remove a variable from the current scope, following upvar links.
    fn remove_var(&mut self, name: &str) {
        if self.frames.is_empty() {
            self.globals.remove(name);
            return;
        }
        let frame_idx = self.frames.len() - 1;
        if let Some(link) = self.frames[frame_idx].upvars.get(name).cloned() {
            match link {
                UpvarLink::Global(gname) => {
                    self.globals.remove(&gname);
                }
                UpvarLink::Frame { frame_index, var_name } => {
                    if let Some(f) = self.frames.get_mut(frame_index) {
                        f.locals.remove(&var_name);
                    }
                }
            }
        } else {
            self.frames[frame_idx].locals.remove(name);
        }
    }

    // ── public API ─────────────────────────────────────────────

    pub fn get_var(&self, name: &str) -> Result<&Value> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.resolve_var(&full_key).ok_or_else(|| Error::var_not_found(name))
        } else {
            self.resolve_var(name).ok_or_else(|| Error::var_not_found(name))
        }
    }

    pub fn set_var(&mut self, name: &str, value: Value) -> Result<Value> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.store_var(&full_key, value.clone());
            // Ensure the array base key exists in the same scope
            if self.resolve_var(array_name).is_none() {
                self.store_var(array_name, Value::empty());
            }
        } else {
            self.store_var(name, value.clone());
        }
        Ok(value)
    }

    pub fn unset_var(&mut self, name: &str) -> Result<()> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.remove_var(&full_key);
        } else {
            self.remove_var(name);
        }
        Ok(())
    }

    pub fn var_exists(&self, name: &str) -> bool {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.resolve_var(&full_key).is_some()
        } else {
            self.resolve_var(name).is_some()
        }
    }

    pub fn result(&self) -> &Value {
        &self.result
    }

    #[cfg(feature = "std")]
    pub fn set_script_name(&mut self, name: &str) {
        self.script_name = name.to_string();
    }

    #[cfg(feature = "std")]
    pub fn script_name(&self) -> &str {
        &self.script_name
    }
}
