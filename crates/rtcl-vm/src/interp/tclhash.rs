//! Tcl hash-table iteration order — a faithful replay of tclsh 8.6's
//! `Tcl_HashTable` (generic/tclHash.c) mechanics for string keys:
//!
//! - hash: `result += (result << 3) + byte` over UTF-8 bytes (`TclHashStringKey`,
//!   u64 on this platform — `size_t result`);
//! - index: `hash & (numBuckets - 1)`, numBuckets starts at `TCL_SMALL_HASH_TABLE`
//!   (4) and `rebuildSize` at `size * REBUILD_MULTIPLIER` (3 → 12);
//! - insert prepends to the bucket chain; an existing key keeps its position;
//! - on the insert that pushes `entries >= rebuildSize`, the table is rebuilt:
//!   numBuckets and rebuildSize ×4, entries re-inserted (prepended) walking the
//!   old table bucket 0..N, each chain head→tail;
//! - iteration walks buckets 0..N, each chain head→tail.
//!
//! Iteration surfaces (`info commands`, `info vars`, `array names`, `chan
//! names`, `namespace children`, …) replay the table's insertion sequence
//! through [`tcl_order`]; storage maps stay unordered.

/// `TclHashStringKey` — multiply by 9, add byte, over UTF-8 bytes.
pub(crate) fn tcl_hash(key: &str) -> u64 {
    let bytes = key.as_bytes();
    if bytes.is_empty() {
        return 0;
    }
    let mut result = bytes[0] as u64;
    for &b in &bytes[1..] {
        result += (result << 3) + b as u64;
    }
    result
}

/// One emulated Tcl string-key table.  Kept generic enough to also serve
/// `dict info` (bucket statistics).
struct TclTable {
    buckets: Vec<Vec<String>>,
    rebuild: u64,
}

impl TclTable {
    fn new() -> Self {
        Self { buckets: vec![Vec::new(); 4], rebuild: 12 }
    }

    /// Insert in insertion order: existing keys keep their chain position.
    fn insert(&mut self, key: &str) {
        let size = self.buckets.len() as u64;
        let idx = (tcl_hash(key) & (size - 1)) as usize;
        let bucket = &mut self.buckets[idx];
        if bucket.iter().any(|k| k == key) {
            return;
        }
        bucket.insert(0, key.to_string());
        let entries: u64 = self.buckets.iter().map(|b| b.len() as u64).sum();
        if entries >= self.rebuild {
            // Walk old buckets 0..N, each chain head→tail; prepend each
            // entry into the new table (tclHash.c RebuildTable).
            let old: Vec<String> =
                self.buckets.iter().flat_map(|b| b.iter().cloned()).collect();
            let new_size = size * 4;
            let mut fresh = vec![Vec::new(); new_size as usize];
            for k in old {
                let idx = (tcl_hash(&k) & (new_size - 1)) as usize;
                fresh[idx].insert(0, k);
            }
            self.buckets = fresh;
            self.rebuild *= 4;
        }
    }

    fn iteration_order(&self) -> Vec<String> {
        self.buckets.iter().flatten().cloned().collect()
    }
}

/// Replay the Tcl hash-table mechanics over `keys` in insertion order and
/// return them in iteration order.  Keys must be live (callers filter
/// deleted entries first); duplicates are ignored like re-sets in Tcl.
pub(crate) fn tcl_order(keys: &[String]) -> Vec<String> {
    let mut table = TclTable::new();
    for key in keys {
        table.insert(key);
    }
    table.iteration_order()
}

/// Bucket statistics for `dict info`: `(numBuckets, histogram, avg search
/// distance)`.  The histogram counts buckets by chain length (10+ collapsed),
/// as in tclsh's `Tcl_HashStats`.
pub(crate) fn tcl_table_stats(keys: &[String]) -> (u64, [u64; 11], f64) {
    let mut table = TclTable::new();
    for key in keys {
        table.insert(key);
    }
    let mut hist = [0u64; 11];
    let mut entries = 0u64;
    let mut distance = 0u64;
    for b in &table.buckets {
        hist[b.len().min(10) as usize] += 1;
        for (i, _) in b.iter().enumerate() {
            distance += (i + 1) as u64;
        }
        entries += b.len() as u64;
    }
    let avg = if entries > 0 { distance as f64 / entries as f64 } else { 0.0 };
    (table.buckets.len() as u64, hist, avg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oracle_pairs() -> Vec<(&'static str, &'static str)> {
        // (insertion sequence, tclsh 8.6.17 `array names` output)
        vec![
            ("b a c z y", "y a z b c"),
            ("a b c d e", "d e a b c"),
            (
                "one two three four five six seven eight nine ten",
                "six four three eight seven nine five two one ten",
            ),
            (
                "q w e r t y u i o p a s d f g h j k l z x c v b n m",
                "p q a b r c s d t e u v f g w x h y i z j k l m n o",
            ),
            (
                "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen",
                "three fifteen seven one two thirteen four eleven fourteen eight five nine six ten twelve",
            ),
            (
                "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi",
                "xi delta nu gamma zeta iota kappa alpha theta lambda epsilon eta mu beta",
            ),
        ]
    }

    #[test]
    fn matches_tclsh_array_names_order() {
        for (input, expected) in oracle_pairs() {
            let keys: Vec<String> = input.split_whitespace().map(String::from).collect();
            let got = tcl_order(&keys).join(" ");
            assert_eq!(got, expected, "input: {input}");
        }
    }

    #[test]
    fn dict_info_buckets_match_tclsh() {
        // tclsh: dict info [dict create k1 1 ... k12 12] → 16 buckets,
        // 7 empty / 6 with one / 3 with two entries.
        let keys: Vec<String> =
            (1..=12).map(|i| format!("k{i}")).collect();
        let (buckets, hist, avg) = tcl_table_stats(&keys);
        assert_eq!(buckets, 16);
        assert_eq!(hist[0], 7);
        assert_eq!(hist[1], 6);
        assert_eq!(hist[2], 3);
        assert!((avg - 1.25).abs() < 1e-9);
    }
}
