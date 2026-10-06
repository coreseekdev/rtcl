//! Variable-table maps with a tclsh-style fast hash.
//!
//! The interpreter's hot path resolves variable names through several hash
//! probes per command — `get_var`'s locals probe plus the array-set probe,
//! `set_var`'s upvar/array/write triple — which with std's SipHash-1-3
//! default costs ~20ns per short-name probe (~100ns of pure hashing per
//! `incr`).  tclsh's variable tables hash names with an *unkeyed* Jenkins
//! one-at-a-time hash: script-provided names are the same trust level tclsh
//! gives them, and the var tables are small enough that a weak hash's
//! collision behaviour is irrelevant.  [`VarHasher`] is that trade — a
//! multiply-mix (Fx-style) hash over the name bytes, ~2ns per probe.
//!
//! Only the variable tables (globals / locals / their array-name sets /
//! upvar links) use it; keyed tables that face untrusted-bytes-shaped input
//! keep std's SipHash.

// ---------------------------------------------------------------------------
// Hasher
// ---------------------------------------------------------------------------

#[cfg(not(feature = "embedded"))]
mod fx {
    use std::hash::Hasher;

    /// FxHash's mixing constant.
    const SEED: u64 = 0x517c_c1b7_2722_0a95;

    /// Unkeyed multiply-mix hasher for short string keys.
    #[derive(Default, Clone)]
    pub struct VarHasher {
        hash: u64,
    }

    impl VarHasher {
        #[inline]
        fn add(&mut self, word: u64) {
            self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
        }
    }

    // Only `write` + `write_u8` are exercised: `Hash for str` is
    // `write(bytes)` followed by a `write_u8(0xff)` terminator (the same
    // shape `write_str`'s default decomposes into).
    impl Hasher for VarHasher {
        #[inline]
        fn finish(&self) -> u64 {
            self.hash
        }

        fn write(&mut self, bytes: &[u8]) {
            let mut chunks = bytes.chunks_exact(8);
            for c in &mut chunks {
                self.add(u64::from_le_bytes(c.try_into().unwrap()));
            }
            let rem = chunks.remainder();
            if !rem.is_empty() {
                // Zero-padded tail: "abc" and "abc\0" still differ through
                // the str-hash terminator byte.
                let mut buf = [0u8; 8];
                buf[..rem.len()].copy_from_slice(rem);
                self.add(u64::from_le_bytes(buf));
            }
        }

        #[inline]
        fn write_u8(&mut self, i: u8) {
            self.add(i as u64);
        }
    }
}

// ---------------------------------------------------------------------------
// Map aliases
// ---------------------------------------------------------------------------

#[cfg(not(feature = "embedded"))]
pub(crate) use fx::VarHasher;

#[cfg(not(feature = "embedded"))]
pub(crate) type VarMap<V> =
    std::collections::HashMap<String, V, std::hash::BuildHasherDefault<VarHasher>>;

/// Generic Fx-hashed map: same unkeyed fast hash, any key/value.  For
/// interpreter-side hot tables whose keys are interpreter-shaped
/// (script text, names) — std's SipHash cost ~20ns/probe vs ~2ns.
#[cfg(not(feature = "embedded"))]
pub(crate) type FxHashedMap<K, V> =
    std::collections::HashMap<K, V, std::hash::BuildHasherDefault<VarHasher>>;

#[cfg(not(feature = "embedded"))]
pub(crate) type VarSet =
    std::collections::HashSet<String, std::hash::BuildHasherDefault<VarHasher>>;

/// The embedded (no_std) build keeps ordered maps — no hasher machinery.
#[cfg(feature = "embedded")]
pub(crate) type VarMap<V> = alloc::collections::BTreeMap<String, V>;

#[cfg(feature = "embedded")]
pub(crate) type VarSet = alloc::collections::BTreeSet<String>;

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "embedded"))]
    #[test]
    fn map_round_trips_short_names() {
        let mut m = super::VarMap::default();
        for n in ["i", "n", "sum", "x", "accumulator", "a", "ab", "abc", "abcd", "abcdefghij"] {
            m.insert(n.to_string(), n.len() as i64);
        }
        for n in ["i", "n", "sum", "x", "accumulator", "a", "ab", "abc", "abcd", "abcdefghij"] {
            assert_eq!(m.get(n), Some(&(n.len() as i64)), "{n}");
        }
        assert_eq!(m.get("missing"), None);
    }

    #[cfg(not(feature = "embedded"))]
    #[test]
    fn prefix_names_dont_collide_badly() {
        // The tail-padding + terminator must keep length-distinct names
        // distinct (a weak-hash classic: "a\0" vs "a").
        let mut m = super::VarMap::default();
        let names: Vec<String> = (0..64).map(|i| "v".repeat(i)).collect();
        for (i, n) in names.iter().enumerate() {
            m.insert(n.clone(), i as i64);
        }
        for (i, n) in names.iter().enumerate() {
            assert_eq!(m.get(n.as_str()), Some(&(i as i64)), "{n:?}");
        }
    }

    #[test]
    fn set_insert_remove() {
        let mut s = super::VarSet::default();
        s.insert("arr".to_string());
        assert!(s.contains("arr"));
        s.remove("arr");
        assert!(!s.contains("arr"));
    }
}
