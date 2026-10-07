//! 原子写：同目录临时文件 + fsync + rename（spec §4 共享契约「原子落位」）。

use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rtcl_core::error::{Error, ErrorCode, Result};
use rtcl_core::interp::Interp;
use rtcl_core::value::Value;

pub fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let tmp = parent.join(format!(".tmp-{}-{nanos}", std::process::id()));
    let write_all = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.flush()?;
        f.sync_all()?;
        Ok(())
    };
    if let Err(e) = write_all() {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// `atomic-write <path> <content>`
pub fn cmd_atomic_write(_interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 3 {
        return Err(Error::wrong_args("atomic-write", 3, args.len()));
    }
    let path = Path::new(args[1].as_str());
    atomic_write(path, args[2].as_str())
        .map_err(|e| Error::runtime(e.to_string(), ErrorCode::InvalidOp))?;
    Ok(Value::empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rtcl-data-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writes_and_overwrites_atomically() {
        let d = tmpdir("aw");
        let p = d.join("card.yaml");
        atomic_write(&p, "v1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "v1\n");
        atomic_write(&p, "v2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "v2\n");
        // 无临时文件残留
        let leftovers: Vec<_> = std::fs::read_dir(&d).unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "card.yaml").collect();
        assert!(leftovers.is_empty(), "leftover: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_parent_errors_and_cleans() {
        let d = tmpdir("aw2");
        let p = d.join("no-such-dir").join("x.yaml");
        assert!(atomic_write(&p, "x").is_err());
        // 父目录未被半途创建
        assert!(!d.join("no-such-dir").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
