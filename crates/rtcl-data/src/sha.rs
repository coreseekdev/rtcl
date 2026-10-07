//! 十六进制摘要。
//!
//! ## Commands
//!
//! - `sha1 string`
//! - `sha256 string`

use rtcl_core::error::{Error, Result};
use rtcl_core::interp::Interp;
use rtcl_core::value::Value;
use sha1::{Digest, Sha1};
use sha2::Sha256;

pub fn sha1_hex(s: &str) -> String {
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

pub fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

/// `sha1 string` → 40 位十六进制摘要
pub fn cmd_sha1(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args("sha1", 2, args.len()));
    }
    Ok(Value::from_str(&sha1_hex(args[1].as_str())))
}

/// `sha256 string` → 64 位十六进制摘要
pub fn cmd_sha256(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args("sha256", 2, args.len()));
    }
    Ok(Value::from_str(&sha256_hex(args[1].as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_vectors() {
        assert_eq!(sha1_hex(""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn cmd_sha1_happy_path() {
        let mut i = rtcl_core::interp::Interp::new();
        let v = cmd_sha1(&mut i, &[
            Value::from_str("sha1"), Value::from_str("abc"),
        ]).unwrap();
        assert_eq!(v.as_str(), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn cmd_sha256_happy_path() {
        let mut i = rtcl_core::interp::Interp::new();
        let v = cmd_sha256(&mut i, &[
            Value::from_str("sha256"), Value::from_str("abc"),
        ]).unwrap();
        assert_eq!(
            v.as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn cmd_sha_arity_errors() {
        let mut i = rtcl_core::interp::Interp::new();
        assert!(cmd_sha1(&mut i, &[Value::from_str("sha1")]).is_err());
        assert!(cmd_sha1(&mut i, &[
            Value::from_str("sha1"), Value::from_str("a"), Value::from_str("b"),
        ]).is_err());
        assert!(cmd_sha256(&mut i, &[Value::from_str("sha256")]).is_err());
    }
}
