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
use std::rc::Rc;
use std::collections::HashMap;

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

// ── Cached singletons ──────────────────────────────────────────
//
// The VM's hottest paths (`PushEmpty`, `PushBool`, integer loop counters)
// create the same values millions of times.  By caching them as thread-local
// `Rc` singletons we eliminate per-use `Rc::new` + `OnceCell::from`
// overhead — `Value::clone()` becomes a simple reference-count increment.

const SMALL_INT_CACHE_SIZE: usize = 256;
/// Lower bound of the small-int cache: the negatives cover the `-1`
/// step operands loop code pushes per iteration (`expr {$n - 1}`'s
/// PushInt), which would otherwise malloc a fresh `Rc<ValueInner>`
/// every time.
const INT_CACHE_MIN: i64 = -128;

thread_local! {
    static CACHED_EMPTY: Rc<ValueInner> = Rc::new(ValueInner {
        string: OnceCell::from(SmallVec::new()),
        rep: InternalRep::None,
    });

    static CACHED_BOOL_TRUE: Rc<ValueInner> = Rc::new(ValueInner {
        string: OnceCell::from(SmallVec::from_slice(b"1")),
        rep: InternalRep::Bool(true),
    });

    static CACHED_BOOL_FALSE: Rc<ValueInner> = Rc::new(ValueInner {
        string: OnceCell::from(SmallVec::from_slice(b"0")),
        rep: InternalRep::Bool(false),
    });

    /// Cached small integers [-128, 256) — covers loop counters, return
    /// codes, list indices, most Tcl numeric constants, and the negative
    /// step operands of decrementing loops.
    static CACHED_INTS: Vec<Rc<ValueInner>> = (INT_CACHE_MIN..SMALL_INT_CACHE_SIZE as i64)
        .map(|n| {
            Rc::new(ValueInner {
                string: OnceCell::from(SmallVec::from_slice(format_int(n).as_bytes())),
                rep: InternalRep::Int(n),
            })
        })
        .collect();
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
            inner: CACHED_EMPTY.with(Rc::clone),
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
        // Fast path: cached singleton for small integers (negatives
        // included — `-1` sits in every decrementing loop).
        if n >= INT_CACHE_MIN && n < SMALL_INT_CACHE_SIZE as i64 {
            return CACHED_INTS.with(|ints| Value {
                inner: Rc::clone(&ints[(n - INT_CACHE_MIN) as usize]),
            });
        }
        // Lazy string (tclsh: an object born with an int rep carries no
        // string until demanded) — the loop-counter case renders never.
        Value {
            inner: Rc::new(ValueInner {
                string: OnceCell::new(),
                rep: InternalRep::Int(n),
            }),
        }
    }

    /// Create a value from a float
    pub fn from_float(n: f64) -> Self {
        Value {
            inner: Rc::new(ValueInner {
                string: OnceCell::new(),
                rep: InternalRep::Float(n),
            }),
        }
    }

    /// Create a value from a boolean
    pub fn from_bool(b: bool) -> Self {
        Value {
            inner: if b {
                CACHED_BOOL_TRUE.with(Rc::clone)
            } else {
                CACHED_BOOL_FALSE.with(Rc::clone)
            },
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

    /// Create a value from a list of values.
    ///
    /// The string form is generated lazily on first `as_str()` — eagerly
    /// materializing it here retains every parent's serialization forever
    /// in deeply nested lists (`set x [list $x {}]` grew to 24 GB RSS
    /// because each level kept its own full-length string; tclsh keeps
    /// the object tree structural and never stringifies, obj-32.1).
    pub fn from_list(items: &[Value]) -> Self {
        Self::from_list_cached(items.to_vec())
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
                // Handle hex, octal, binary — with optional sign
                // (`-0x1234` -> -4660, Tcl signed radix literals).
                let (neg, body) = match s.strip_prefix('-') {
                    Some(r) => (true, r),
                    None => (false, s.strip_prefix('+').unwrap_or(s)),
                };
                let radix_parse = if let Some(rest) =
                    body.strip_prefix("0x").or_else(|| body.strip_prefix("0X"))
                {
                    i64::from_str_radix(rest, 16).ok()
                } else if let Some(rest) =
                    body.strip_prefix("0o").or_else(|| body.strip_prefix("0O"))
                {
                    i64::from_str_radix(rest, 8).ok()
                } else if let Some(rest) =
                    body.strip_prefix("0b").or_else(|| body.strip_prefix("0B"))
                {
                    i64::from_str_radix(rest, 2).ok()
                } else if body.len() > 1 && body.starts_with('0') {
                    // Legacy octal: `017` is 15 (tclsh); a digit 8/9 makes
                    // the text not-an-integer (`expr 018` errors).
                    let digits = &body[1..];
                    if digits.bytes().all(|d| (b'0'..=b'7').contains(&d)) {
                        return u64::from_str_radix(digits, 8)
                            .ok()
                            .map(|n| n as i64)
                            .map(|n| if neg { n.wrapping_neg() } else { n });
                    }
                    None
                } else {
                    // Plain decimal: parse the original text (sign included).
                    return i64::from_str(s).ok();
                };
                if neg {
                    radix_parse.map(|n| n.wrapping_neg())
                } else {
                    radix_parse
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
                // Tcl accepts Inf / -Inf / NaN (any case) as numeric strings.
                match s.to_ascii_lowercase().as_str() {
                    "inf" | "infinity" => return Some(f64::INFINITY),
                    "-inf" | "-infinity" => return Some(f64::NEG_INFINITY),
                    "nan" => return Some(f64::NAN),
                    _ => {}
                }
                f64::from_str(s).ok()
            }
        }
    }

    /// Try to get as boolean
    ///
    /// Tcl rules (TclGetBooleanFromObj): any numeric value coerces by
    /// zero-ness (2.5 → true, 0x10 → true, 00 → false); otherwise the
    /// string must be an unambiguous prefix of true/false/yes/no/on/off
    /// (`t`, `ye`, `of` are valid; `o` alone is ambiguous → None).
    pub fn as_bool(&self) -> Option<bool> {
        match &self.inner.rep {
            InternalRep::Bool(b) => Some(*b),
            _ => {
                if let Some(i) = self.as_int() {
                    return Some(i != 0);
                }
                if let Some(f) = self.as_float() {
                    return if f.is_nan() { None } else { Some(f != 0.0) };
                }
                let s = self.to_str();
                let s = s.trim();
                if s.is_empty() {
                    return None;
                }
                let lower = s.to_ascii_lowercase();
                let mut candidates = ["true", "false", "yes", "no", "on", "off"]
                    .iter()
                    .filter(|word| word.starts_with(lower.as_str()));
                let first = *candidates.next()?;
                if candidates.next().is_some() {
                    return None; // ambiguous prefix ("o")
                }
                Some(matches!(first, "true" | "yes" | "on"))
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

    /// Like `as_list()`, but malformed list strings yield Tcl's parse
    /// error (message + `::errorCode`) instead of `None`.
    pub fn as_list_strict(&self) -> std::result::Result<Vec<Value>, ListParseError> {
        match &self.inner.rep {
            InternalRep::List(items) => Ok(items.clone()),
            InternalRep::Dict(map) => {
                Ok(map.iter().flat_map(|(k, v)| [Value::from_str(k), v.clone()]).collect())
            }
            _ => {
                let s = self.to_str();
                parse_list_full(&s)
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

    /// Get a mutable reference to the list items (COW) — the
    /// `as_dict_mut` mirror for lists.  In-place when this handle is
    /// the value's sole owner (the interpreter's mutation fast path
    /// takes the variable out of its slot first, tclsh's refcnt==1
    /// mutation); otherwise the `ValueInner` is cloned once.
    pub fn as_list_mut(&mut self) -> Option<&mut Vec<Value>> {
        if !matches!(&self.inner.rep, InternalRep::List(_)) {
            return None;
        }
        let inner = Rc::make_mut(&mut self.inner);
        inner.string = OnceCell::new(); // invalidate string
        match &mut inner.rep {
            InternalRep::List(items) => Some(items),
            _ => None,
        }
    }

    /// Overwrite the integer rep in place (COW) — the `incr` fast path,
    /// tclsh's TclIncrObj on a refcnt==1 object: no allocation, and the
    /// cached string rendering is dropped instead of eagerly rebuilt.
    pub fn set_int_rep(&mut self, n: i64) {
        let inner = Rc::make_mut(&mut self.inner);
        inner.rep = InternalRep::Int(n);
        inner.string = OnceCell::new(); // invalidate string
    }

    /// Do both handles point at the same `ValueInner` allocation?
    ///
    /// Pointer identity, not string equality — the loop-body memo keys on
    /// "is this the exact script Value the interpreter handed out last
    /// time".  Clones share the allocation and still compare equal; a
    /// value rebuilt from text compares different even when the text
    /// matches, so a stale memo can never be hit through a look-alike.
    pub fn same_allocation(&self, other: &Value) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
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

    // ── Type introspection (mirrors jimtcl's typePtr) ──────────

    /// Return the name of the current internal representation type.
    ///
    /// Mirrors jimtcl's `typePtr->name` — answers "what type **is** this
    /// value right now?" without triggering any conversion.
    ///
    /// Possible return values: `"string"`, `"int"`, `"float"`, `"bool"`,
    /// `"list"`, `"dict"`.
    pub fn type_name(&self) -> &'static str {
        match &self.inner.rep {
            InternalRep::None => "string",
            InternalRep::Int(_) => "int",
            InternalRep::Float(_) => "float",
            InternalRep::Bool(_) => "bool",
            InternalRep::List(_) => "list",
            InternalRep::Dict(_) => "dict",
        }
    }

    /// Borrow the list **only** if the internal rep is already `List`.
    ///
    /// Unlike `as_list()` this never parses a string into a list,
    /// mirroring the semantics of `as_dict_ref()`.
    pub fn as_list_ref(&self) -> Option<&[Value]> {
        match &self.inner.rep {
            InternalRep::List(items) => Some(items.as_slice()),
            _ => None,
        }
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
///
/// This is a faithful port of Tcl 8.6's `TclScanElement` /
/// `TclConvertElement` / `Tcl_Merge` (generic/tclUtil.c, COMPAT=1): each
/// element is emitted verbatim, brace-quoted, or backslash-escaped
/// according to its contents; elements are joined with single spaces; a
/// leading `#` is quoted only for the first element.
fn serialize_list(items: &[Value]) -> String {
    let mut out: Vec<u8> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(b' ');
        }
        convert_element(&mut out, item.as_str().as_bytes(), i == 0);
    }
    // Safe: item strings are valid UTF-8, we only insert ASCII bytes and
    // never split a multi-byte sequence.
    String::from_utf8(out).unwrap_or_default()
}

/// Tcl's list whitespace: the only bytes that separate list elements.
pub fn is_tcl_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// How a list element must be quoted (mirrors Tcl's CONVERT_* flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConvMode {
    /// No quoting needed.
    None,
    /// Enclose in braces.
    Brace,
    /// Backslash-escape every special character (including braces).
    Escape,
    /// Backslash-escape specials except braces (historical Tcl mode used
    /// when quoting is needed solely due to `]` or `"`).
    Mask,
}

/// Port of Tcl's `TclScanElement`: decide how `s` must be quoted.
fn scan_element(s: &[u8]) -> ConvMode {
    if s.is_empty() {
        return ConvMode::Brace;
    }
    let mut forbid_none = false;
    let mut require_escape = false;
    let mut prefer_escape = false;
    let mut prefer_brace = false;
    let mut nesting: i64 = 0;

    if s[0] == b'{' || s[0] == b'"' {
        // Leading character would be misread as list syntax.
        forbid_none = true;
        prefer_brace = true;
    }

    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'{' => nesting += 1,
            b'}' => {
                nesting -= 1;
                if nesting < 0 {
                    require_escape = true;
                }
            }
            b']' | b'"' => {
                forbid_none = true;
                prefer_escape = true;
            }
            b'[' | b'$' | b';' => {
                forbid_none = true;
                prefer_brace = true;
            }
            b'\\' => {
                if i + 1 == s.len() {
                    // Final backslash cannot be brace-quoted.
                    require_escape = true;
                } else if s[i + 1] == b'\n' {
                    // Backslash-newline cannot be brace-quoted.
                    require_escape = true;
                    i += 1;
                } else if matches!(s[i + 1], b'{' | b'}' | b'\\') {
                    i += 1;
                }
                forbid_none = true;
                prefer_brace = true;
            }
            c if is_tcl_space(c) => {
                forbid_none = true;
                prefer_brace = true;
            }
            _ => {}
        }
        i += 1;
    }

    if nesting != 0 {
        require_escape = true;
    }
    if require_escape {
        ConvMode::Escape
    } else if forbid_none {
        if prefer_escape && !prefer_brace {
            ConvMode::Mask
        } else {
            ConvMode::Brace
        }
    } else {
        ConvMode::None
    }
}

/// Port of Tcl's `TclConvertElement`: append the list representation of
/// `s` to `out`. `quote_hash` mirrors `!TCL_DONT_QUOTE_HASH` (only true
/// for the first element of a list).
fn convert_element(out: &mut Vec<u8>, s: &[u8], quote_hash: bool) {
    if s.is_empty() {
        out.extend_from_slice(b"{}");
        return;
    }
    let mut mode = scan_element(s);
    let mut start = 0;
    if quote_hash && s[0] == b'#' {
        if mode == ConvMode::Escape {
            out.extend_from_slice(b"\\#");
            start = 1;
        } else {
            mode = ConvMode::Brace;
        }
    }
    match mode {
        ConvMode::None => out.extend_from_slice(&s[start..]),
        ConvMode::Brace => {
            out.push(b'{');
            out.extend_from_slice(&s[start..]);
            out.push(b'}');
        }
        ConvMode::Escape | ConvMode::Mask => {
            for &b in &s[start..] {
                match b {
                    b']' | b'[' | b'$' | b';' | b' ' | b'\\' | b'"' => {
                        out.push(b'\\');
                        out.push(b);
                    }
                    b'{' | b'}' => {
                        if mode == ConvMode::Escape {
                            out.push(b'\\');
                        }
                        out.push(b);
                    }
                    0x0c => out.extend_from_slice(b"\\f"),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    b'\r' => out.extend_from_slice(b"\\r"),
                    b'\t' => out.extend_from_slice(b"\\t"),
                    0x0b => out.extend_from_slice(b"\\v"),
                    _ => out.push(b),
                }
            }
        }
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

/// Parse a string as a Tcl list. Returns `None` for malformed lists
/// (unbalanced braces/quotes, junk after a braced or quoted element).
fn parse_list(s: &str) -> Option<Vec<Value>> {
    parse_list_full(s).ok()
}

/// Error produced when parsing a malformed Tcl list string.
///
/// `message` matches Tcl 8.6's error text (e.g. "unmatched open brace in
/// list"); `code` is the corresponding `::errorCode` value
/// (e.g. "TCL VALUE LIST BRACE").
#[derive(Debug, Clone)]
pub struct ListParseError {
    pub message: String,
    pub code: &'static str,
}

/// Parse a string as a Tcl list, reporting Tcl's error on malformed input.
///
/// This is a port of Tcl 8.6's `FindElement` (generic/tclUtil.c): elements
/// are separated by ASCII whitespace; braced elements keep their contents
/// verbatim (backslashes only affect brace counting); quoted and bare
/// elements undergo backslash substitution (`TclCopyAndCollapse`).
pub fn parse_list_full(s: &str) -> std::result::Result<Vec<Value>, ListParseError> {
    parse_elements(s, false)
}

/// Parse a string as a Tcl dictionary's element sequence, reporting Tcl's
/// dict-specific error texts and errorCodes. `tclDictObj.c` shares the
/// list element scanner but says "dict element in braces followed by ..."
/// and raises TCL VALUE DICTIONARY JUNK/BRACE/QUOTE.
pub fn parse_dict_full(s: &str) -> std::result::Result<Vec<Value>, ListParseError> {
    parse_elements(s, true)
}

fn parse_elements(s: &str, dict: bool) -> std::result::Result<Vec<Value>, ListParseError> {
    let b = s.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    loop {
        while i < b.len() && is_tcl_space(b[i]) {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let (elem, next) = find_element(s, i, dict)?;
        result.push(Value::from_str(&elem));
        i = next;
    }
    Ok(result)
}

fn junk_error(s: &str, i: usize, kind: &str, dict: bool) -> ListParseError {
    // Tcl reports up to 20 bytes of the offending text.
    let b = s.as_bytes();
    let mut j = i;
    while j < b.len() && !is_tcl_space(b[j]) && j < i + 20 {
        j += 1;
    }
    while !s.is_char_boundary(j) {
        j -= 1;
    }
    let (what, code) = if dict {
        ("dict", "TCL VALUE DICTIONARY JUNK")
    } else {
        ("list", "TCL VALUE LIST JUNK")
    };
    ListParseError {
        message: format!(
            "{} element in {} followed by \"{}\" instead of space",
            what,
            kind,
            &s[i..j]
        ),
        code,
    }
}

/// Locate the list element starting at byte `start` (which must not be
/// whitespace). Returns the element value and the position just past any
/// whitespace following the element.
fn find_element(
    s: &str,
    start: usize,
    dict: bool,
) -> std::result::Result<(String, usize), ListParseError> {
    let b = s.as_bytes();
    let mut i = start;

    if b[i] == b'{' {
        // Braced element: contents are literal; backslashes only matter
        // for brace counting.
        i += 1;
        let elem_start = i;
        let mut depth = 1i32;
        while i < b.len() {
            match b[i] {
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    depth -= 1;
                    i += 1;
                    if depth == 0 {
                        if i >= b.len() || is_tcl_space(b[i]) {
                            let mut j = i;
                            while j < b.len() && is_tcl_space(b[j]) {
                                j += 1;
                            }
                            return Ok((s[elem_start..i - 1].to_string(), j));
                        }
                        return Err(junk_error(s, i, "braces", dict));
                    }
                }
                b'\\' => {
                    let (n, _) = scan_backslash(s, i);
                    i += n;
                }
                _ => i += 1,
            }
        }
        return Err(if dict {
            ListParseError {
                message: "unmatched open brace in dict".to_string(),
                code: "TCL VALUE DICTIONARY BRACE",
            }
        } else {
            ListParseError {
                message: "unmatched open brace in list".to_string(),
                code: "TCL VALUE LIST BRACE",
            }
        });
    }

    if b[i] == b'"' {
        // Quoted element: backslash substitution applies.
        i += 1;
        let elem_start = i;
        let mut has_backslash = false;
        while i < b.len() {
            match b[i] {
                b'"' => {
                    let raw = &s[elem_start..i];
                    i += 1;
                    if i >= b.len() || is_tcl_space(b[i]) {
                        let mut j = i;
                        while j < b.len() && is_tcl_space(b[j]) {
                            j += 1;
                        }
                        let v = if has_backslash { collapse(raw) } else { raw.to_string() };
                        return Ok((v, j));
                    }
                    return Err(junk_error(s, i, "quotes", dict));
                }
                b'\\' => {
                    has_backslash = true;
                    let (n, _) = scan_backslash(s, i);
                    i += n;
                }
                _ => i += 1,
            }
        }
        return Err(if dict {
            ListParseError {
                message: "unmatched open quote in dict".to_string(),
                code: "TCL VALUE DICTIONARY QUOTE",
            }
        } else {
            ListParseError {
                message: "unmatched open quote in list".to_string(),
                code: "TCL VALUE LIST QUOTE",
            }
        });
    }

    // Bare element: ends at whitespace; backslash substitution applies.
    let elem_start = i;
    let mut has_backslash = false;
    while i < b.len() && !is_tcl_space(b[i]) {
        if b[i] == b'\\' {
            has_backslash = true;
            let (n, _) = scan_backslash(s, i);
            i += n;
        } else {
            i += 1;
        }
    }
    let raw = &s[elem_start..i];
    let v = if has_backslash { collapse(raw) } else { raw.to_string() };
    Ok((v, i))
}

/// Substitute all backslash sequences in `s` (Tcl's `TclCopyAndCollapse`).
fn collapse(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            let (n, rep) = scan_backslash(s, i);
            out.push_str(&rep);
            i += n;
        } else {
            let ch_len = utf8_len(b[i]);
            out.push_str(&s[i..i + ch_len]);
            i += ch_len;
        }
    }
    out
}

/// Length in bytes of the UTF-8 sequence starting with byte `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >= 0xf0 {
        4
    } else if b >= 0xe0 {
        3
    } else {
        2
    }
}

/// Scan the backslash sequence starting at `s[i]` (which must be `\\`),
/// following Tcl's `TclParseBackslash` rules. Returns the number of bytes
/// consumed (including the backslash) and the substituted text.
fn scan_backslash(s: &str, i: usize) -> (usize, String) {
    let b = s.as_bytes();
    debug_assert!(b[i] == b'\\');
    if i + 1 >= b.len() {
        // Lone trailing backslash stays a backslash.
        return (1, "\\".to_string());
    }
    let c = b[i + 1];
    match c {
        b'a' => (2, "\x07".to_string()),
        b'b' => (2, "\x08".to_string()),
        b'f' => (2, "\x0c".to_string()),
        b'n' => (2, "\n".to_string()),
        b'r' => (2, "\r".to_string()),
        b't' => (2, "\t".to_string()),
        b'v' => (2, "\x0b".to_string()),
        b'x' => {
            let mut val: u32 = 0;
            let mut n = 0;
            while n < 2 && i + 2 + n < b.len() && (b[i + 2 + n] as char).is_ascii_hexdigit() {
                val = val * 16 + (b[i + 2 + n] as char).to_digit(16).unwrap_or(0);
                n += 1;
            }
            if n == 0 {
                (2, "x".to_string())
            } else {
                let ch = char::from_u32(val & 0xff).unwrap_or('\u{fffd}');
                (2 + n, ch.to_string())
            }
        }
        b'u' | b'U' => {
            let max_digits = if c == b'u' { 4 } else { 8 };
            let mut val: u32 = 0;
            let mut n = 0;
            while n < max_digits && i + 2 + n < b.len() && (b[i + 2 + n] as char).is_ascii_hexdigit()
            {
                val = val
                    .saturating_mul(16)
                    .saturating_add((b[i + 2 + n] as char).to_digit(16).unwrap_or(0));
                n += 1;
            }
            if n == 0 {
                return (2, (c as char).to_string());
            }
            // Combine a \u high surrogate with a following \u low surrogate.
            if c == b'u'
                && n == 4
                && (0xd800..0xdc00).contains(&val)
                && i + 2 + n + 1 < b.len()
                && b[i + 2 + n] == b'\\'
                && b[i + 2 + n + 1] == b'u'
            {
                let lo_start = i + 2 + n + 2;
                let mut low: u32 = 0;
                let mut m = 0;
                while m < 4 && lo_start + m < b.len() && (b[lo_start + m] as char).is_ascii_hexdigit()
                {
                    low = low * 16 + (b[lo_start + m] as char).to_digit(16).unwrap_or(0);
                    m += 1;
                }
                if m == 4 && (0xdc00..0xe000).contains(&low) {
                    let combined = ((val & 0x3ff) << 10 | (low & 0x3ff)) + 0x10000;
                    let ch = char::from_u32(combined).unwrap_or('\u{fffd}');
                    return (2 + n + 2 + m, ch.to_string());
                }
            }
            if (0xd800..0xe000).contains(&val) {
                return (2 + n, '\u{fffd}'.to_string());
            }
            let ch = char::from_u32(val).unwrap_or('\u{fffd}');
            (2 + n, ch.to_string())
        }
        b'0'..=b'7' => {
            let mut val: u32 = (c - b'0') as u32;
            let mut n = 1;
            while n < 3 && i + 1 + n < b.len() && matches!(b[i + 1 + n], b'0'..=b'7') {
                // Tcl only takes a third octal digit when the value stays < 256.
                if n == 2 && val >= 0x20 {
                    break;
                }
                val = val * 8 + (b[i + 1 + n] - b'0') as u32;
                n += 1;
            }
            let ch = char::from_u32(val & 0xff).unwrap_or('\u{fffd}');
            (1 + n, ch.to_string())
        }
        b'\n' => {
            // Backslash-newline and following spaces/tabs collapse to one space.
            let mut j = i + 2;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            (j - i, " ".to_string())
        }
        _ => {
            // Unknown escape: the backslash quotes the following character.
            let ch_len = utf8_len(c);
            let end = (i + 1 + ch_len).min(b.len());
            (1 + (end - (i + 1)), s[i + 1..end].to_string())
        }
    }
}

/// Format an integer for Tcl
fn format_int(n: i64) -> String {
    format!("{}", n)
}

/// Format a float for Tcl
fn format_float(n: f64) -> String {
    // tclsh renders non-finite floats as Inf / -Inf / NaN, not Rust's
    // lowercase "inf"/"NaN".
    if n.is_infinite() {
        return if n > 0.0 { "Inf".to_string() } else { "-Inf".to_string() };
    }
    if n.is_nan() {
        return "NaN".to_string();
    }
    // tclsh (8.6.17, verified): shortest round-trip digits, laid out in
    // fixed notation when the decimal exponent is in -4..=16, exponent
    // form otherwise ("1e+17", "1e-5" — exponent without leading zeros);
    // integral fixed values keep a trailing ".0".
    let e = format!("{:e}", n); // shortest repr, e.g. "-6.02e23"
    let (mant, exp_str) = e.split_once('e').unwrap();
    let exp: i32 = exp_str.parse().unwrap_or(0);
    let neg = mant.starts_with('-');
    let mant = mant.trim_start_matches('-');
    let mut digits: String = mant.chars().filter(|c| *c != '.').collect();
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    if digits.is_empty() {
        digits.push('0');
    }
    let mut body = String::new();
    if (-4..=16).contains(&exp) {
        let nd = digits.len() as i32;
        if exp >= nd - 1 {
            body.push_str(&digits);
            for _ in 0..(exp - (nd - 1)) {
                body.push('0');
            }
            body.push_str(".0");
        } else if exp >= 0 {
            body.push_str(&digits[..(exp + 1) as usize]);
            body.push('.');
            body.push_str(&digits[(exp + 1) as usize..]);
        } else {
            body.push_str("0.");
            for _ in 0..(-exp - 1) {
                body.push('0');
            }
            body.push_str(&digits);
        }
    } else {
        body.push_str(&digits[..1]);
        if digits.len() > 1 {
            body.push('.');
            body.push_str(&digits[1..]);
        }
        body.push('e');
        if exp < 0 {
            body.push('-');
        } else {
            body.push('+');
        }
        body.push_str(&exp.abs().to_string());
    }
    if neg {
        body.insert(0, '-');
    }
    body
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

    #[test]
    fn test_serialize_matches_tcl() {
        // Oracle outputs from tclsh 8.6.17 `list`.
        let cases: &[(&[&str], &str)] = &[
            (&["a}b"], "a\\}b"),
            (&["a{b"], "a\\{b"),
            (&["a{b}c"], "a{b}c"),
            (&["a{b}c{d"], "a\\{b\\}c\\{d"),
            (&["a b"], "{a b}"),
            (&["a\nb"], "{a\nb}"),
            (&[""], "{}"),
            (&["\\"], "\\\\"),
            (&["\\n"], "{\\n}"),
            (&["a\"b"], "a\\\"b"),
            (&["a]b"], "a\\]b"),
            (&["a[b"], "{a[b}"),
            (&["a$b"], "{a$b}"),
            (&["a;b"], "{a;b}"),
            (&["{"], "\\{"),
            (&["}"], "\\}"),
            (&["#foo"], "{#foo}"),
            (&["a", "#foo"], "a #foo"),
            (&["#", "#"], "{#} #"),
            (&["#{"], "\\#\\{"),
            (&["a", "#{"], "a #\\{"),
            (&["\""], "{\"}"),
            (&["a] b"], "{a] b}"),
            (&["{a}"], "{{a}}"),
            (&["a\x0bb"], "{a\x0bb}"),
        ];
        for (items, expected) in cases {
            let vals: Vec<Value> = items.iter().map(|s| Value::from_str(s)).collect();
            assert_eq!(serialize_list(&vals), *expected, "items: {:?}", items);
        }
    }

    #[test]
    fn test_list_round_trip() {
        let items = [
            "a}b", "{a}", "", "\\", "\\n", "a\nb", "\u{0}x", "#hash",
            "a\"b", " lead", "trail ", "x{y}z", "中", "\u{1} ",
        ];
        for item in items {
            let list = Value::from_list(&[Value::from_str(item)]);
            let parsed = list.as_list().unwrap();
            assert_eq!(parsed.len(), 1, "serialized: {}", list.as_str());
            assert_eq!(parsed[0].as_str(), item, "serialized: {}", list.as_str());
        }
    }

    #[test]
    fn test_parse_malformed_lists() {
        assert!(Value::from_str("{").as_list().is_none());
        assert!(Value::from_str("a {b").as_list().is_none());
        assert!(Value::from_str("\"abc").as_list().is_none());
        assert!(Value::from_str("{a}b").as_list().is_none());
        assert!(Value::from_str("\"a\"b").as_list().is_none());
        let err = Value::from_str("{").as_list_strict().unwrap_err();
        assert_eq!(err.message, "unmatched open brace in list");
        assert_eq!(err.code, "TCL VALUE LIST BRACE");
        let err = Value::from_str("\"a").as_list_strict().unwrap_err();
        assert_eq!(err.message, "unmatched open quote in list");
        let err = Value::from_str("{a}b").as_list_strict().unwrap_err();
        assert_eq!(err.message, "list element in braces followed by \"b\" instead of space");
        assert_eq!(err.code, "TCL VALUE LIST JUNK");
    }

    #[test]
    fn test_parse_backslash_collapse() {
        let v = Value::from_str("a\\}b");
        let list = v.as_list().unwrap();
        assert_eq!(list[0].as_str(), "a}b");
        let v = Value::from_str("a\\nb");
        assert_eq!(v.as_list().unwrap()[0].as_str(), "a\nb");
        let v = Value::from_str("a\\x41b");
        assert_eq!(v.as_list().unwrap()[0].as_str(), "aAb");
        let v = Value::from_str("q\\q");
        assert_eq!(v.as_list().unwrap()[0].as_str(), "qq");
        // Trailing backslash in a bare element is kept.
        let v = Value::from_str("a\\");
        assert_eq!(v.as_list().unwrap()[0].as_str(), "a\\");
        // Backslash-newline folds to a single space.
        let v = Value::from_str("a\\\n  b");
        assert_eq!(v.as_list().unwrap()[0].as_str(), "a b");
    }

    #[test]
    fn test_parse_braced_literal_backslashes() {
        // Inside braces, backslashes are literal (only affect brace counting).
        let v = Value::from_str("{a\\}b}");
        let list = v.as_list().unwrap();
        assert_eq!(list[0].as_str(), "a\\}b");
        // `{\}` is an unmatched brace: the \} does not close the element.
        assert!(Value::from_str("{\\}").as_list().is_none());
    }
}
