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
use std::cell::OnceCell;
use std::collections::HashMap;
use std::rc::Rc;

use indexmap::IndexMap;
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
    /// Cached dict — supports both ordered (insertion-order) and unordered modes.
    Dict(DictMap),
    /// No internal representation yet
    None,
}

// ── DictMap ────────────────────────────────────────────────────

/// Dictionary map supporting both ordered (insertion-order) and unordered modes.
///
/// - `Ordered`: backed by `IndexMap`, preserves insertion order (standard Tcl dict semantics)
/// - `Unordered`: backed by `HashMap`, no order guarantee (faster for large dicts)
#[derive(Debug, Clone)]
pub enum DictMap {
    Ordered(IndexMap<String, Value>),
    Unordered(HashMap<String, Value>),
}

impl Default for DictMap {
    fn default() -> Self { DictMap::Ordered(IndexMap::new()) }
}

impl DictMap {
    pub fn ordered() -> Self { DictMap::Ordered(IndexMap::new()) }
    pub fn unordered() -> Self { DictMap::Unordered(HashMap::new()) }

    pub fn ordered_with_capacity(cap: usize) -> Self {
        DictMap::Ordered(IndexMap::with_capacity(cap))
    }
    pub fn unordered_with_capacity(cap: usize) -> Self {
        DictMap::Unordered(HashMap::with_capacity(cap))
    }

    pub fn is_ordered(&self) -> bool { matches!(self, DictMap::Ordered(_)) }

    /// Build a new DictMap with the given ordering mode from an iterator.
    pub fn from_iter_with_order<I>(ordered: bool, iter: I) -> Self
    where
        I: IntoIterator<Item = (String, Value)>,
    {
        if ordered {
            DictMap::Ordered(iter.into_iter().collect())
        } else {
            DictMap::Unordered(iter.into_iter().collect())
        }
    }

    /// Create an empty DictMap matching the ordering mode of `self`.
    pub fn empty_like(&self, capacity: usize) -> Self {
        if self.is_ordered() {
            DictMap::ordered_with_capacity(capacity)
        } else {
            DictMap::unordered_with_capacity(capacity)
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self { DictMap::Ordered(m) => m.get(key), DictMap::Unordered(m) => m.get(key) }
    }

    pub fn insert(&mut self, key: String, value: Value) -> Option<Value> {
        match self { DictMap::Ordered(m) => m.insert(key, value), DictMap::Unordered(m) => m.insert(key, value) }
    }

    /// Remove a key (preserves order for ordered maps).
    pub fn shift_remove(&mut self, key: &str) -> Option<Value> {
        match self { DictMap::Ordered(m) => m.shift_remove(key), DictMap::Unordered(m) => m.remove(key) }
    }

    pub fn contains_key(&self, key: &str) -> bool {
        match self { DictMap::Ordered(m) => m.contains_key(key), DictMap::Unordered(m) => m.contains_key(key) }
    }

    pub fn len(&self) -> usize {
        match self { DictMap::Ordered(m) => m.len(), DictMap::Unordered(m) => m.len() }
    }

    pub fn is_empty(&self) -> bool {
        match self { DictMap::Ordered(m) => m.is_empty(), DictMap::Unordered(m) => m.is_empty() }
    }

    pub fn keys(&self) -> DictKeys<'_> {
        match self {
            DictMap::Ordered(m) => DictKeys::Ordered(m.keys()),
            DictMap::Unordered(m) => DictKeys::Unordered(m.keys()),
        }
    }

    pub fn values(&self) -> DictValues<'_> {
        match self {
            DictMap::Ordered(m) => DictValues::Ordered(m.values()),
            DictMap::Unordered(m) => DictValues::Unordered(m.values()),
        }
    }

    pub fn iter(&self) -> DictIter<'_> {
        match self {
            DictMap::Ordered(m) => DictIter::Ordered(m.iter()),
            DictMap::Unordered(m) => DictIter::Unordered(m.iter()),
        }
    }
}

impl Extend<(String, Value)> for DictMap {
    fn extend<I: IntoIterator<Item = (String, Value)>>(&mut self, iter: I) {
        match self { DictMap::Ordered(m) => m.extend(iter), DictMap::Unordered(m) => m.extend(iter) }
    }
}

impl IntoIterator for DictMap {
    type Item = (String, Value);
    type IntoIter = DictIntoIter;
    fn into_iter(self) -> Self::IntoIter {
        match self {
            DictMap::Ordered(m) => DictIntoIter::Ordered(m.into_iter()),
            DictMap::Unordered(m) => DictIntoIter::Unordered(m.into_iter()),
        }
    }
}

impl<'a> IntoIterator for &'a DictMap {
    type Item = (&'a String, &'a Value);
    type IntoIter = DictIter<'a>;
    fn into_iter(self) -> Self::IntoIter { self.iter() }
}

// ── Iterator types ─────────────────────────────────────────────

pub enum DictIter<'a> {
    Ordered(indexmap::map::Iter<'a, String, Value>),
    Unordered(std::collections::hash_map::Iter<'a, String, Value>),
}
impl<'a> Iterator for DictIter<'a> {
    type Item = (&'a String, &'a Value);
    fn next(&mut self) -> Option<Self::Item> {
        match self { DictIter::Ordered(i) => i.next(), DictIter::Unordered(i) => i.next() }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self { DictIter::Ordered(i) => i.size_hint(), DictIter::Unordered(i) => i.size_hint() }
    }
}

pub enum DictIntoIter {
    Ordered(indexmap::map::IntoIter<String, Value>),
    Unordered(std::collections::hash_map::IntoIter<String, Value>),
}
impl Iterator for DictIntoIter {
    type Item = (String, Value);
    fn next(&mut self) -> Option<Self::Item> {
        match self { DictIntoIter::Ordered(i) => i.next(), DictIntoIter::Unordered(i) => i.next() }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self { DictIntoIter::Ordered(i) => i.size_hint(), DictIntoIter::Unordered(i) => i.size_hint() }
    }
}

pub enum DictKeys<'a> {
    Ordered(indexmap::map::Keys<'a, String, Value>),
    Unordered(std::collections::hash_map::Keys<'a, String, Value>),
}
impl<'a> Iterator for DictKeys<'a> {
    type Item = &'a String;
    fn next(&mut self) -> Option<Self::Item> {
        match self { DictKeys::Ordered(i) => i.next(), DictKeys::Unordered(i) => i.next() }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self { DictKeys::Ordered(i) => i.size_hint(), DictKeys::Unordered(i) => i.size_hint() }
    }
}

pub enum DictValues<'a> {
    Ordered(indexmap::map::Values<'a, String, Value>),
    Unordered(std::collections::hash_map::Values<'a, String, Value>),
}
impl<'a> Iterator for DictValues<'a> {
    type Item = &'a Value;
    fn next(&mut self) -> Option<Self::Item> {
        match self { DictValues::Ordered(i) => i.next(), DictValues::Unordered(i) => i.next() }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self { DictValues::Ordered(i) => i.size_hint(), DictValues::Unordered(i) => i.size_hint() }
    }
}

/// Inner data shared via `Rc`.
///
/// Corresponds to jimtcl's `Jim_Obj` minus the free-list pointers —
/// Rust's allocator handles recycling.
#[derive(Debug, Clone)]
struct ValueInner {
    /// String representation — lazily materialized via `OnceCell`.
    /// Empty cell means it will be generated on first `as_str()` access
    /// from `rep` (like jimtcl's `bytes == NULL`).
    string: OnceCell<SmallVec<[u8; INLINE_SIZE]>>,
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
                string: OnceCell::from(SmallVec::new()),
                rep: InternalRep::None,
            }),
        }
    }

    /// Create a value from a string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Value {
            inner: Rc::new(ValueInner {
                string: OnceCell::from(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::None,
            }),
        }
    }

    /// Create a value from an integer
    pub fn from_int(n: i64) -> Self {
        let s = format_int(n);
        Value {
            inner: Rc::new(ValueInner {
                string: OnceCell::from(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::Int(n),
            }),
        }
    }

    /// Create a value from a float
    pub fn from_float(n: f64) -> Self {
        let s = format_float(n);
        Value {
            inner: Rc::new(ValueInner {
                string: OnceCell::from(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::Float(n),
            }),
        }
    }

    /// Create a value from a boolean
    pub fn from_bool(b: bool) -> Self {
        let s = if b { "1" } else { "0" };
        Value {
            inner: Rc::new(ValueInner {
                string: OnceCell::from(SmallVec::from_slice(s.as_bytes())),
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
                string: OnceCell::new(), // lazy — generated on first as_str()
                rep: InternalRep::List(items),
            }),
        }
    }

    /// Create a value directly from a cached dict.
    ///
    /// The string representation is lazily generated on first `as_str()`.
    pub fn from_dict_cached(entries: DictMap) -> Self {
        Value {
            inner: Rc::new(ValueInner {
                string: OnceCell::new(),
                rep: InternalRep::Dict(entries),
            }),
        }
    }

    /// Create a dict value from key-value pairs (ordered).
    pub fn from_dict_pairs(pairs: &[(Value, Value)]) -> Self {
        let mut map = IndexMap::with_capacity(pairs.len());
        for (k, v) in pairs {
            map.insert(k.as_str().to_string(), v.clone());
        }
        Value {
            inner: Rc::new(ValueInner {
                string: OnceCell::new(),
                rep: InternalRep::Dict(DictMap::Ordered(map)),
            }),
        }
    }

    /// Create a value from a list of values (serializes to string eagerly
    /// for backward compatibility).
    pub fn from_list(items: &[Value]) -> Self {
        let s = serialize_list(items);
        Value {
            inner: Rc::new(ValueInner {
                string: OnceCell::from(SmallVec::from_slice(s.as_bytes())),
                rep: InternalRep::List(items.to_vec()),
            }),
        }
    }

    // ── Accessors ──────────────────────────────────────────────

    /// Get the string representation.
    ///
    /// If the string has not been materialized yet (lazy value), it is
    /// auto-generated from the internal representation via `OnceCell`.
    pub fn as_str(&self) -> &str {
        let bytes = self.inner.string.get_or_init(|| {
            match &self.inner.rep {
                InternalRep::List(items) => {
                    SmallVec::from_slice(serialize_list(items).as_bytes())
                }
                InternalRep::Dict(map) => {
                    SmallVec::from_slice(serialize_dict(map).as_bytes())
                }
                InternalRep::Int(n) => SmallVec::from_slice(format_int(*n).as_bytes()),
                InternalRep::Float(n) => SmallVec::from_slice(format_float(*n).as_bytes()),
                InternalRep::Bool(b) => {
                    SmallVec::from_slice(if *b { b"1" } else { b"0" })
                }
                InternalRep::None => SmallVec::new(),
            }
        });
        // Safety: we always store valid UTF-8
        unsafe { core::str::from_utf8_unchecked(bytes) }
    }

    /// Ensure the string representation is materialized.
    /// With `OnceCell`, `as_str()` auto-materializes, so this is now a convenience.
    pub fn ensure_string(&mut self) {
        let _ = self.as_str();
    }

    /// Get the string representation, materializing it if needed.
    /// Now simply delegates to `as_str()` since `OnceCell` handles lazy init.
    pub fn to_str(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(self.as_str())
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
            InternalRep::Dict(map) => {
                Some(map.iter().flat_map(|(k, v)| [Value::from_str(k), v.clone()]).collect())
            }
            _ => {
                let s = self.to_str();
                parse_list(&s)
            }
        }
    }

    /// Get a reference to the dict's DictMap if the internal rep is Dict.
    /// Zero-copy — does not clone.
    pub fn as_dict_ref(&self) -> Option<&DictMap> {
        match &self.inner.rep {
            InternalRep::Dict(map) => Some(map),
            _ => None,
        }
    }

    /// Get a mutable reference to the dict's DictMap (COW).
    pub fn as_dict_mut(&mut self) -> Option<&mut DictMap> {
        if !matches!(&self.inner.rep, InternalRep::Dict(_)) {
            return None;
        }
        let inner = Rc::make_mut(&mut self.inner);
        inner.string = OnceCell::new(); // invalidate string
        match &mut inner.rep {
            InternalRep::Dict(map) => Some(map),
            _ => None,
        }
    }

    /// Parse or return a dict as an owned DictMap.
    pub fn as_dict(&self) -> Option<DictMap> {
        match &self.inner.rep {
            InternalRep::Dict(map) => Some(map.clone()),
            InternalRep::List(items) => {
                if items.len() % 2 != 0 { return None; }
                let mut map = DictMap::ordered_with_capacity(items.len() / 2);
                for c in items.chunks(2) {
                    map.insert(c[0].as_str().to_string(), c[1].clone());
                }
                Some(map)
            }
            _ => {
                let s = self.to_str();
                let list = parse_list(&s)?;
                if list.len() % 2 != 0 { return None; }
                let mut map = DictMap::ordered_with_capacity(list.len() / 2);
                for c in list.chunks(2) {
                    map.insert(c[0].as_str().to_string(), c[1].clone());
                }
                Some(map)
            }
        }
    }

    /// Borrow the dict without cloning when the internal rep is already Dict.
    /// Returns `Cow::Borrowed` for zero-copy access, `Cow::Owned` when parsing is needed.
    pub fn as_dict_cow(&self) -> Option<std::borrow::Cow<'_, DictMap>> {
        match &self.inner.rep {
            InternalRep::Dict(map) => Some(std::borrow::Cow::Borrowed(map)),
            InternalRep::List(items) => {
                if items.len() % 2 != 0 { return None; }
                let mut map = DictMap::ordered_with_capacity(items.len() / 2);
                for c in items.chunks(2) {
                    map.insert(c[0].as_str().to_string(), c[1].clone());
                }
                Some(std::borrow::Cow::Owned(map))
            }
            _ => {
                let s = self.to_str();
                let list = parse_list(&s)?;
                if list.len() % 2 != 0 { return None; }
                let mut map = DictMap::ordered_with_capacity(list.len() / 2);
                for c in list.chunks(2) {
                    map.insert(c[0].as_str().to_string(), c[1].clone());
                }
                Some(std::borrow::Cow::Owned(map))
            }
        }
    }

    /// Check if the value is empty
    pub fn is_empty(&self) -> bool {
        if let Some(bytes) = self.inner.string.get() {
            bytes.is_empty()
        } else {
            match &self.inner.rep {
                InternalRep::List(items) => items.is_empty(),
                InternalRep::Dict(map) => map.is_empty(),
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
fn serialize_dict(map: &DictMap) -> String {
    let items: Vec<Value> = map
        .iter()
        .flat_map(|(k, v)| [Value::from_str(k), v.clone()])
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
