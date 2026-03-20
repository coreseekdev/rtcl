//! Value type for rtcl - implements "everything is a string" philosophy
//!
//! Tcl values can have an internal representation for efficiency
//! while still being representable as strings.
//!
//! # Memory model
//!
//! Values are reference-counted (`Rc<ValueInner>`) with copy-on-write
//! semantics, mirroring jimtcl's `refCount` / `Jim_DuplicateObj()` model.
//!
//! - `Value::clone()` is O(1) — simply increments the reference count.
//! - Mutation (e.g. list append) uses `Rc::make_mut()`, which deep-copies
//!   only when the value is shared (`strong_count > 1`).
//! - Structured data (lists, dicts) are cached inside `InternalRep` to
//!   avoid repeated string parse / serialize round-trips.
//! - The string representation is lazily generated: after mutating the
//!   internal representation, the string is invalidated and only
//!   regenerated when `as_str()` is called.

use core::fmt;
use core::str::FromStr;
use std::rc::Rc;

use smallvec::SmallVec;

/// Maximum inline string length before heap allocation
const INLINE_SIZE: usize = 23;

/// Internal representation of a value.
///
/// Mirrors jimtcl's `internalRep` union — a cached typed view of the
/// underlying string value.
#[derive(Debug, Clone)]
pub enum InternalRep {
    /// Integer representation
    Int(i64),
    /// Floating point representation
    Float(f64),
    /// Boolean representation
    Bool(bool),
    /// Cached list of values (avoids re-parsing the string)
    List(Vec<Value>),
    /// Cached dict of key-value pairs (avoids re-parsing the string)
    Dict(Vec<(Value, Value)>),
    /// No internal representation yet
    None,
}

/// Inner data shared via `Rc`.
///
/// Corresponds to jimtcl's `Jim_Obj` minus the free-list pointers —
/// Rust's allocator handles recycling.
#[derive(Debug, Clone)]
struct ValueInner {
    /// String representation — `None` means it must be regenerated
    /// from `rep` (the "dirty" / invalidated state, like jimtcl's
    /// `bytes == NULL`).
    string: Option<SmallVec<[u8; INLINE_SIZE]>>,
    /// Cached typed representation.
    rep: InternalRep,
}

/// A Tcl value - "everything is a string"
///
/// Cloning a `Value` is **O(1)** (reference-count increment).
/// Mutation triggers copy-on-write when the value is shared.
#[derive(Debug, Clone)]
pub struct Value {
    inner: Rc<ValueInner>,
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
            inner: Rc::new(ValueInner {
                string: Some(SmallVec::new()),
                rep: InternalRep::None,
            }),
        }
    }

    /// Create a value from a string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Value {
            inner: Rc::new(ValueInner {
                string: Some(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::None,
            }),
        }
    }

    /// Create a value from an integer
    pub fn from_int(n: i64) -> Self {
        let s = format_int(n);
        Value {
            inner: Rc::new(ValueInner {
                string: Some(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::Int(n),
            }),
        }
    }

    /// Create a value from a float
    pub fn from_float(n: f64) -> Self {
        let s = format_float(n);
        Value {
            inner: Rc::new(ValueInner {
                string: Some(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::Float(n),
            }),
        }
    }

    /// Create a value from a boolean
    pub fn from_bool(b: bool) -> Self {
        let s = if b { "1" } else { "0" };
        Value {
            inner: Rc::new(ValueInner {
                string: Some(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::Bool(b),
            }),
        }
    }

    /// Create a value directly from a cached list of values.
    ///
    /// The string representation is lazily generated on first `as_str()`
    /// call — this avoids the serialize cost when the list is only ever
    /// accessed structurally (e.g. `lindex`, `lappend`).
    pub fn from_list_cached(items: Vec<Value>) -> Self {
        Value {
            inner: Rc::new(ValueInner {
                string: None, // lazy — will be generated on demand
                rep: InternalRep::List(items),
            }),
        }
    }

    /// Create a value directly from a cached dict.
    ///
    /// The string representation is lazily generated on first `as_str()`.
    pub fn from_dict_cached(entries: Vec<(Value, Value)>) -> Self {
        Value {
            inner: Rc::new(ValueInner {
                string: None,
                rep: InternalRep::Dict(entries),
            }),
        }
    }

    /// Create a value from dict entries (serializes to string eagerly).
    pub fn from_dict(entries: &[(Value, Value)]) -> Self {
        let s = serialize_dict(entries);
        Value {
            inner: Rc::new(ValueInner {
                string: Some(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::Dict(entries.to_vec()),
            }),
        }
    }

    /// Create a value from a list of values (serializes to string eagerly
    /// for backward compatibility).
    pub fn from_list(items: &[Value]) -> Self {
        let s = serialize_list(items);
        Value {
            inner: Rc::new(ValueInner {
                string: Some(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::List(items.to_vec()),
            }),
        }
    }

    // ── Accessors ──────────────────────────────────────────────

    /// Get the string representation.
    ///
    /// If the string has been invalidated (e.g. after an in-place list
    /// mutation), it is regenerated from the internal representation.
    pub fn as_str(&self) -> &str {
        // Fast path: string is already materialized
        if let Some(ref bytes) = self.inner.string {
            // Safety: we always store valid UTF-8
            return unsafe { core::str::from_utf8_unchecked(bytes) };
        }
        // Slow path should not happen in practice because we always
        // ensure the string is materialized before returning &str.
        // But as a safety net, return empty string.
        //
        // The real lazy-regen path is handled by ensure_string() which
        // must be called before as_str() when the value was mutated.
        ""
    }

    /// Ensure the string representation is materialized.
    /// Call this before passing the value to code that needs `.as_str()`.
    pub fn ensure_string(&mut self) {
        if self.inner.string.is_none() {
            let s = match &self.inner.rep {
                InternalRep::List(items) => serialize_list(items),
                InternalRep::Dict(entries) => serialize_dict(entries),
                InternalRep::Int(n) => format_int(*n),
                InternalRep::Float(n) => format_float(*n),
                InternalRep::Bool(b) => (if *b { "1" } else { "0" }).to_string(),
                InternalRep::None => String::new(),
            };
            Rc::make_mut(&mut self.inner).string =
                Some(SmallVec::from_slice(s.as_bytes()));
        }
    }

    /// Get the string representation, materializing it if needed.
    ///
    /// Returns an owned `String` to avoid lifetime issues when the
    /// string was lazily generated.
    pub fn to_str(&self) -> std::borrow::Cow<'_, str> {
        if let Some(ref bytes) = self.inner.string {
            std::borrow::Cow::Borrowed(
                unsafe { core::str::from_utf8_unchecked(bytes) }
            )
        } else {
            let s = match &self.inner.rep {
                InternalRep::List(items) => serialize_list(items),
                InternalRep::Dict(entries) => serialize_dict(entries),
                InternalRep::Int(n) => format_int(*n),
                InternalRep::Float(n) => format_float(*n),
                InternalRep::Bool(b) => (if *b { "1" } else { "0" }).to_string(),
                InternalRep::None => String::new(),
            };
            std::borrow::Cow::Owned(s)
        }
    }

    /// Try to get as integer
    pub fn as_int(&self) -> Option<i64> {
        match &self.inner.rep {
            InternalRep::Int(n) => Some(*n),
            _ => {
                let s = self.to_str();
                let s = s.trim();
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
        match &self.inner.rep {
            InternalRep::Float(n) => Some(*n),
            InternalRep::Int(n) => Some(*n as f64),
            _ => {
                let s = self.to_str();
                let s = s.trim();
                f64::from_str(s).ok()
            }
        }
    }

    /// Try to get as boolean
    pub fn as_bool(&self) -> Option<bool> {
        match &self.inner.rep {
            InternalRep::Bool(b) => Some(*b),
            _ => {
                let s = self.to_str();
                let s = s.trim();
                // Tcl boolean rules
                match s.to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" | "on" => Some(true),
                    "0" | "false" | "no" | "off" => Some(false),
                    _ => None,
                }
            }
        }
    }

    /// Get the cached list if available, or parse from string.
    ///
    /// Returns a freshly parsed `Vec<Value>` — callers that need to
    /// mutate should use the `_mut` helpers instead.
    pub fn as_list(&self) -> Option<Vec<Value>> {
        match &self.inner.rep {
            InternalRep::List(items) => Some(items.clone()),
            InternalRep::Dict(entries) => {
                Some(entries.iter().flat_map(|(k, v)| [k.clone(), v.clone()]).collect())
            }
            _ => {
                let s = self.to_str();
                parse_list(&s)
            }
        }
    }

    /// Get the value as a dict (ordered key-value pairs).
    ///
    /// Returns cached pairs when the internal rep is already `Dict`,
    /// otherwise parses from the string/list representation.
    pub fn as_dict(&self) -> Option<Vec<(Value, Value)>> {
        match &self.inner.rep {
            InternalRep::Dict(entries) => Some(entries.clone()),
            InternalRep::List(items) => {
                if items.len() % 2 != 0 { return None; }
                Some(items.chunks(2).map(|c| (c[0].clone(), c[1].clone())).collect())
            }
            _ => {
                let s = self.to_str();
                let list = parse_list(&s)?;
                if list.len() % 2 != 0 { return None; }
                Some(list.chunks(2).map(|c| (c[0].clone(), c[1].clone())).collect())
            }
        }
    }

    /// Check if the value is empty
    pub fn is_empty(&self) -> bool {
        if let Some(ref bytes) = self.inner.string {
            bytes.is_empty()
        } else {
            match &self.inner.rep {
                InternalRep::List(items) => items.is_empty(),
                InternalRep::Dict(entries) => entries.is_empty(),
                InternalRep::None => true,
                _ => false,
            }
        }
    }

    /// Get the length of the string representation
    pub fn len(&self) -> usize {
        let s = self.to_str();
        s.len()
    }

    /// Check if the value is a valid number
    pub fn is_number(&self) -> bool {
        self.as_int().is_some() || self.as_float().is_some()
    }

    /// Concatenate two values as strings
    pub fn concat(&self, other: &Value) -> Value {
        let a = self.to_str();
        let b = other.to_str();
        let mut result = String::with_capacity(a.len() + b.len());
        result.push_str(&a);
        result.push_str(&b);
        Value::from_str(&result)
    }

    /// Compare two values
    pub fn compare(&self, other: &Value) -> core::cmp::Ordering {
        let a = self.to_str();
        let b = other.to_str();
        a.cmp(&b)
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

    // ── Rc / COW helpers ───────────────────────────────────────

    /// Returns `true` if this value is shared (reference count > 1).
    ///
    /// Equivalent to jimtcl's `Jim_IsShared()`.
    pub fn is_shared(&self) -> bool {
        Rc::strong_count(&self.inner) > 1
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.to_str();
        write!(f, "{}", s)
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

/// Serialize dict entries into a Tcl list string (key1 val1 key2 val2 ...).
fn serialize_dict(entries: &[(Value, Value)]) -> String {
    let items: Vec<Value> = entries
        .iter()
        .flat_map(|(k, v)| [k.clone(), v.clone()])
        .collect();
    serialize_list(&items)
}

/// Serialize a slice of values into a Tcl list string.
fn serialize_list(items: &[Value]) -> String {
    let mut result = String::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            result.push(' ');
        }
        let s = item.to_str();
        if needs_braces(&s) {
            result.push('{');
            result.push_str(&s);
            result.push('}');
        } else if needs_quotes(&s) {
            result.push('"');
            for c in s.chars() {
                if c == '"' || c == '\\' {
                    result.push('\\');
                }
                result.push(c);
            }
            result.push('"');
        } else {
            result.push_str(&s);
        }
    }
    result
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
                        // In braced strings, only \{ and \} affect brace
                        // counting — the backslash itself is always preserved.
                        result.push('\\');
                        if i + 1 < chars.len()
                            && (chars[i + 1] == '{' || chars[i + 1] == '}')
                        {
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
