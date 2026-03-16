//! Tcl index parsing — handles `end`, `end-N`, `end+N`, and plain integers.

/// Parse a Tcl index string relative to a collection of the given `length`.
///
/// Supports:
/// - Plain integer (0-based): `"3"` → `Some(3)`
/// - `"end"` → last element
/// - `"end-N"` → last minus N
/// - `"end+N"` → last plus N (clamped to length-1)
///
/// Returns `None` if the string is not a valid index.
pub fn parse_index(s: &str, length: usize) -> Option<usize> {
    let s = s.trim();
    if s == "end" {
        return Some(length.saturating_sub(1));
    }
    if let Some(rest) = s.strip_prefix("end-") {
        let offset: usize = rest.parse().ok()?;
        return Some(length.saturating_sub(1 + offset));
    }
    if let Some(rest) = s.strip_prefix("end+") {
        let offset: usize = rest.parse().ok()?;
        return Some(length.saturating_sub(1).saturating_add(offset));
    }
    let n: i64 = s.parse().ok()?;
    if n < 0 {
        None
    } else {
        Some(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_int() {
        assert_eq!(parse_index("0", 5), Some(0));
        assert_eq!(parse_index("3", 5), Some(3));
        assert_eq!(parse_index("7", 5), Some(7)); // out of bounds, caller clamps
    }

    #[test]
    fn test_end() {
        assert_eq!(parse_index("end", 5), Some(4));
        assert_eq!(parse_index("end", 1), Some(0));
        assert_eq!(parse_index("end", 0), Some(0)); // saturating
    }

    #[test]
    fn test_end_minus() {
        assert_eq!(parse_index("end-0", 5), Some(4));
        assert_eq!(parse_index("end-1", 5), Some(3));
        assert_eq!(parse_index("end-4", 5), Some(0));
        assert_eq!(parse_index("end-10", 5), Some(0)); // saturating
    }

    #[test]
    fn test_end_plus() {
        assert_eq!(parse_index("end+0", 5), Some(4));
        assert_eq!(parse_index("end+1", 5), Some(5));
    }

    #[test]
    fn test_negative() {
        assert_eq!(parse_index("-1", 5), None);
    }

    #[test]
    fn test_invalid() {
        assert_eq!(parse_index("abc", 5), None);
        assert_eq!(parse_index("", 5), None);
    }
}
