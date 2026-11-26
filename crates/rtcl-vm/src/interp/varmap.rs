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
    use std::sync::OnceLock;

    /// FxHash's mixing constant.
    const SEED: u64 = 0x517c_c1b7_2722_0a95;

    /// Per-process random seed (S39，审计：无密钥哈希可预构造冲突键).
    /// std 的 RandomState 每**实例**持一对随机 SipHash 键——对固定消息
    /// 求一次 finish 即得到进程内一次性的伪随机值，不引入任何依赖。
    /// 同进程内所有表同种子（序确定性保持，双引擎/sweep 一致）；跨进
    /// 程不可预测，预构造的冲突键不再跨运行成立。迭代序已实证不承载
    /// 输出（P6 的 fmix64 换序后 judge/probes/sweep 全绿）。
    fn process_seed() -> u64 {
        static SEED: OnceLock<u64> = OnceLock::new();
        *SEED.get_or_init(|| {
            use std::hash::{BuildHasher, Hash, Hasher};
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            0x7274_636c_5f73_3339_u64.hash(&mut h); // "rtcl_s39"
            let a = h.finish();
            a.wrapping_mul(0x9e37_79b9_7f4a_7c15)
        })
    }

    /// Unkeyed multiply-mix hasher for short string keys.
    #[derive(Clone)]
    pub struct VarHasher {
        hash: u64,
        seed: u64,
    }

    impl Default for VarHasher {
        fn default() -> Self {
            Self { hash: 0, seed: process_seed() }
        }
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
            // Per-process seed XOR (S39) before the Murmur3 fmix64
            // avalanche (the same finalizer value.rs's dict-key FxHasher
            // has run since its 300k-probe-chain incident).  The raw
            // multiply-mix leaves the bucket index (HashMap's LOW bits)
            // dependent only on the first ~2 bytes of a name: generated
            // families that share a prefix and length — `k000000..kN` —
            // collapsed into ~32 of 131072 buckets (chains of 1800+) at
            // 60k variables; the variable tables degenerated exactly
            // like the dict tables did.  fmix64 is a bijection, so
            // distinct names stay distinct; the seed XOR keeps the
            // distribution (P6) while decorrelating runs; ~4 ops per
            // finish, far under the probe chains it removes (P6).
            let mut x = self.hash ^ self.seed;
            x ^= x >> 33;
            x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
            x ^= x >> 33;
            x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
            x ^= x >> 33;
            x
        }

        fn write(&mut self, bytes: &[u8]) {
            let (chunks, rem) = bytes.as_chunks::<8>();
            for c in chunks {
                self.add(u64::from_le_bytes(*c));
            }
            if !rem.is_empty() {
                // Zero-padded little-endian tail, assembled without a
                // scratch buffer: the old `copy_from_slice` into an
                // `[u8; 8]` showed up as `memcpy` inside the hash of
                // every 1-4 byte variable name (~2% of the OO-call
                // profile).  Length-distinct names stay distinct (the
                // terminator keeps "a" apart from "ab"); a NUL suffix
                // collides with its prefix's padded tail ("abc" vs
                // "abc\0"), but HashMap's key equality absorbs that —
                // worst case one extra probe, never a wrong hit.
                let mut tail = 0u64;
                for (i, &b) in rem.iter().enumerate() {
                    tail |= (b as u64) << (8 * i);
                }
                self.add(tail);
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
