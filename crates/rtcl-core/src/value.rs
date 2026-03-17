//! Value type for rtcl - implements "everything is a string" philosophy
//!
//! Tcl values can have an internal representation for efficiency
//! while still being representable as strings.

use core::fmt;
use core::str::FromStr;

use smallvec::SmallVec;

/// Maximum inline string length before heap allocation
const INLINE_SIZE: usize = 23;

/// Internal representation of a value
#[derive(Debug, Clone, Copy)]
pub enum InternalRep {
    /// Integer representation
    Int(i64),
    /// Floating point representation
    Float(f64),
    /// Boolean representation
    Bool(bool),
    /// List representation (indices into parent string)
    List,
    /// No internal representation yet
    None,
}

/// A Tcl value - "everything is a string"
#[derive(Debug, Clone)]
pub struct Value {
    /// String representation (always present)
    string: SmallVec<[u8; INLINE_SIZE]>,
    /// Internal representation (cached)
    internal: InternalRep,
}

impl Default for Value {
    fn default() -> Self {
        Self::empty()
    }
}

impl Value {
    /// Create an empty value
    pub fn empty() -> Self {
        Value {
            string: SmallVec::new(),
            internal: InternalRep::None,
            
        }
    }

    /// Create a value from a string
    pub fn from_str(s: &str) -> Self {
        Value {
            string: SmallVec::from_slice(s.as_bytes()),
            internal: InternalRep::None,
            
        }
    }

    /// Create a value from an integer
    pub fn from_int(n: i64) -> Self {
        let s = format_int(n);
        let mut string = SmallVec::new();
        string.extend_from_slice(s.as_bytes());
        Value {
            string,
            internal: InternalRep::Int(n),
            
        }
    }

    /// Create a value from a float
    pub fn from_float(n: f64) -> Self {
        let s = format_float(n);
        let mut string = SmallVec::new();
        string.extend_from_slice(s.as_bytes());
        Value {
            string,
            internal: InternalRep::Float(n),
            
        }
    }

    /// Create a value from a boolean
    pub fn from_bool(b: bool) -> Self {
        let s = if b { "1" } else { "0" };
        Value {
            string: SmallVec::from_slice(s.as_bytes()),
            internal: InternalRep::Bool(b),
            
        }
    }

    /// Create a value from a list of values
    pub fn from_list(items: &[Value]) -> Self {
        let mut result = String::new();
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                result.push(' ');
            }
            // Quote if necessary
            let s = item.as_str();
            if needs_braces(s) {
                result.push('{');
                result.push_str(s);
                result.push('}');
            } else if needs_quotes(s) {
                result.push('"');
                for c in s.chars() {
                    if c == '"' || c == '\\' {
                        result.push('\\');
                    }
                    result.push(c);
                }
                result.push('"');
            } else {
                result.push_str(s);
            }
        }
        Value::from_str(&result)
    }

    /// Get the string representation
    pub fn as_str(&self) -> &str {
        // Safety: we always store valid UTF-8
        unsafe { core::str::from_utf8_unchecked(&self.string) }
    }

    /// Try to get as integer
    pub fn as_int(&self) -> Option<i64> {
        match self.internal {
            InternalRep::Int(n) => Some(n),
            _ => {
                let s = self.as_str().trim();
                // Handle hex, octal, binary
                if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                    i64::from_str_radix(rest, 16).ok()
                } else if let Some(rest) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) {
                    i64::from_str_radix(rest, 8).ok()
                } else if let Some(rest) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
                    i64::from_str_radix(rest, 2).ok()
                } else {
                    i64::from_str(s).ok()
                }
            }
        }
    }

    /// Try to get as float
    pub fn as_float(&self) -> Option<f64> {
        match self.internal {
            InternalRep::Float(n) => Some(n),
            InternalRep::Int(n) => Some(n as f64),
            _ => {
                let s = self.as_str().trim();
                f64::from_str(s).ok()
            }
        }
    }

    /// Try to get as boolean
    pub fn as_bool(&self) -> Option<bool> {
        match self.internal {
            InternalRep::Bool(b) => Some(b),
            _ => {
                let s = self.as_str().trim();
                // Tcl boolean rules
                match s.to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" | "on" => Some(true),
                    "0" | "false" | "no" | "off" => Some(false),
                    _ => None,
                }
            }
        }
    }

    /// Parse as a list
    pub fn as_list(&self) -> Option<Vec<Value>> {
        // Use the parser to parse as a list
        let s = self.as_str();
        parse_list(s)
    }

    /// Check if the value is empty
    pub fn is_empty(&self) -> bool {
        self.string.is_empty()
    }

    /// Get the length of the string representation
    pub fn len(&self) -> usize {
        self.string.len()
    }

    /// Check if the value is a valid number
    pub fn is_number(&self) -> bool {
        self.as_int().is_some() || self.as_float().is_some()
    }

    /// Concatenate two values as strings
    pub fn concat(&self, other: &Value) -> Value {
        let mut result = String::with_capacity(self.len() + other.len());
        result.push_str(self.as_str());
        result.push_str(other.as_str());
        Value::from_str(&result)
    }

    /// Compare two values
    pub fn compare(&self, other: &Value) -> core::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }

    /// Compare two values numerically if possible
    pub fn compare_numeric(&self, other: &Value) -> Option<core::cmp::Ordering> {
        match (self.as_float(), other.as_float()) {
            (Some(a), Some(b)) => Some(a.partial_cmp(&b)?),
            _ => None,
        }
    }

    /// Check if the value is true (for conditionals)
    pub fn is_true(&self) -> bool {
        self.as_bool().unwrap_or_else(|| !self.is_empty())
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::from_str(s)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::from_str(&s)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::from_int(n)
    }
}

impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::from_int(n as i64)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::from_bool(b)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::from_float(n)
    }
}

/// Check if a string needs braces for list representation
fn needs_braces(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let mut brace_depth = 0i32;
    for c in s.chars() {
        match c {
            '{' => brace_depth += 1,
            '}' => {
                brace_depth -= 1;
                if brace_depth < 0 {
                    return true;
                }
            }
            ' ' | '\t' | '\n' | '\r' | ';' | '"' | '\\' | '[' | ']' | '$' => {
                return true;
            }
            _ => {}
        }
    }
    brace_depth != 0
}

/// Check if a string needs quotes for list representation
fn needs_quotes(s: &str) -> bool {
    s.contains('"') || s.contains('\\')
}

/// Parse a string as a Tcl list
fn parse_list(s: &str) -> Option<Vec<Value>> {
    let mut result = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Skip whitespace
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }

        // Parse element
        let (elem, new_i) = parse_list_element(&chars, i)?;
        result.push(Value::from_str(&elem));
        i = new_i;
    }

    Some(result)
}

/// Parse a single list element
fn parse_list_element(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut i = start;
    let mut result = String::new();

    if i >= chars.len() {
        return None;
    }

    match chars[i] {
        '{' => {
            // Braced element
            i += 1;
            let mut brace_depth = 1;
            while i < chars.len() && brace_depth > 0 {
                match chars[i] {
                    '{' => {
                        brace_depth += 1;
                        result.push('{');
                    }
                    '}' => {
                        brace_depth -= 1;
                        if brace_depth > 0 {
                            result.push('}');
                        }
                    }
                    '\\' => {
                        if i + 1 < chars.len() {
                            i += 1;
                            result.push(chars[i]);
                        }
                    }
                    c => result.push(c),
                }
                i += 1;
            }
        }
        '"' => {
            // Quoted element
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                result.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1; // Skip closing quote
            }
        }
        _ => {
            // Unquoted element
            while i < chars.len() && !chars[i].is_whitespace() {
                result.push(chars[i]);
                i += 1;
            }
        }
    }

    Some((result, i))
}

/// Format an integer for Tcl
fn format_int(n: i64) -> String {
    format!("{}", n)
}

/// Format a float for Tcl
fn format_float(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{:.1}", n)
    } else {
        format!("{}", n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_from_str() {
        let v = Value::from_str("hello");
        assert_eq!(v.as_str(), "hello");
    }

    #[test]
    fn test_value_from_int() {
        let v = Value::from_int(42);
        assert_eq!(v.as_str(), "42");
        assert_eq!(v.as_int(), Some(42));
    }

    #[test]
    fn test_value_from_bool() {
        let v = Value::from_bool(true);
        assert_eq!(v.as_str(), "1");
        assert_eq!(v.as_bool(), Some(true));
    }

    #[test]
    fn test_list_parsing() {
        let v = Value::from_str("a b c");
        let list = v.as_list().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].as_str(), "a");
        assert_eq!(list[1].as_str(), "b");
        assert_eq!(list[2].as_str(), "c");
    }

    #[test]
    fn test_braced_list() {
        let v = Value::from_str("{a b} {c d}");
        let list = v.as_list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].as_str(), "a b");
        assert_eq!(list[1].as_str(), "c d");
    }
}

/// Quote a string for safe use as a Tcl word in a command.
/// Uses braces when the string contains special chars.
pub fn tcl_quote(s: &str) -> String {
    if s.is_empty() {
        return "{}".to_string();
    }
    if !needs_braces(s) {
        return s.to_string();
    }
    format!("{{{}}}", s)
}
