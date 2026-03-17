//! Variable access methods on [`Interp`].

use super::util::split_array_ref;
use super::Interp;
use crate::error::{Error, Result};
use crate::value::Value;

impl Interp {
    pub fn get_var(&self, name: &str) -> Result<&Value> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.get(&full_key).ok_or_else(|| Error::var_not_found(name))
        } else {
            self.vars.get(name).ok_or_else(|| Error::var_not_found(name))
        }
    }

    pub fn set_var(&mut self, name: &str, value: Value) -> Result<Value> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.insert(full_key, value.clone());
            if !self.vars.contains_key(array_name) {
                self.vars.insert(array_name.to_string(), Value::empty());
            }
        } else {
            self.vars.insert(name.to_string(), value.clone());
        }
        Ok(value)
    }

    pub fn unset_var(&mut self, name: &str) -> Result<()> {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.remove(&full_key);
        } else {
            self.vars.remove(name);
        }
        Ok(())
    }

    pub fn var_exists(&self, name: &str) -> bool {
        if let Some((array_name, index)) = split_array_ref(name) {
            let full_key = format!("{}({})", array_name, index);
            self.vars.contains_key(&full_key)
        } else {
            self.vars.contains_key(name)
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
