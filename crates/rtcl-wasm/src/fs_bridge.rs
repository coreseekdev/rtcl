//! JsFs — the JS-side [`HostFs`] data plane (HANDOFF §10quinter).
//!
//! The host registers a JS object via `RtclHandle::set_fs(fs)`; its
//! methods are the data plane for `open`/`file`/`source`/`glob`/`cd`/
//! `pwd` (memory fs for demos, OPFS-sync-access-handle in a worker for
//! production).  The channel PROTOCOL face (buffering/translation/
//! reflected machinery) stays native Rust — see host.rs.
//!
//! Thread-local registries carry the JS objects (js_sys types are not
//! Send; the Rust side stores only u64 ids — the same pattern as
//! js_command.rs).  Byte transfer uses the latin-1 string trick
//! (`charCodeAt & 0xff` / `String.fromCharCode`) — fine for the Tcl
//! demo surface; binary payloads over OPFS go through typed arrays in
//! the worker form (v2).
//!
//! JS object contract (all may throw; the message becomes the Tcl error):
//! ```text
//! fs.open(path, mode)      -> file handle object (or throws)
//! fs.stat(path)            -> {size, mtime, atime, dir, file} | null
//! fs.access(path)          -> {read, write, exec}
//! fs.mkdir(path, all)
//! fs.delete(path)
//! fs.rename(src, dst)
//! fs.copy(src, dst)
//! fs.readdir(path)         -> [names]
//! fs.pwd()                 -> string
//! fs.cd(path)
//! fs.readfile(path)        -> string
//! fs.tempfile(prefix)      -> string
//! file handle:
//!   read(n) -> string|null   readline() -> string|null
//!   readall() -> string      write(s)
//!   flush()                  seek(pos, whence)  // "set"|"cur"|"end"
//!   tell() -> number         eof() -> bool      close()
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use rtcl_vm::channel::Channel;
use rtcl_vm::host::HostFs;

thread_local! {
    static FS_OBJECTS: RefCell<HashMap<u64, js_sys::Object>> = RefCell::new(HashMap::new());
    static FILE_OBJECTS: RefCell<HashMap<u64, js_sys::Object>> = RefCell::new(HashMap::new());
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
}

fn next_id() -> u64 {
    NEXT_ID.with(|n| n.fetch_add(1, Ordering::Relaxed))
}

fn js_err(e: &JsValue) -> io::Error {
    io::Error::new(io::ErrorKind::Other, super::js_command::js_error_message(e))
}

fn call_unit(obj: &js_sys::Object, method: &str, args: &[&JsValue]) -> io::Result<()> {
    let f = js_sys::Reflect::get(obj, &JsValue::from_str(method))
        .map_err(|e| js_err(&e))?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, format!("fs.{} is not a function", method)))?;
    let argv: Vec<JsValue> = args.iter().map(|a| (*a).clone()).collect();
    f.apply(&JsValue::NULL, &js_sys::Array::from(&js_sys::Array::from_iter(argv.into_iter())))
        .map_err(|e| js_err(&e))?;
    Ok(())
}

fn call_ret(obj: &js_sys::Object, method: &str, args: &[&JsValue]) -> io::Result<JsValue> {
    let f = js_sys::Reflect::get(obj, &JsValue::from_str(method))
        .map_err(|e| js_err(&e))?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, format!("fs.{} is not a function", method)))?;
    let argv: Vec<JsValue> = args.iter().map(|a| (*a).clone()).collect();
    f.apply(&JsValue::NULL, &js_sys::Array::from(&js_sys::Array::from_iter(argv.into_iter())))
        .map_err(|e| js_err(&e))
}

/// Registered JS filesystem — Send-safe handle (object lives in TLS).
pub struct JsFs {
    id: u64,
}

impl JsFs {
    /// Register `obj` as this handle's data plane; returns the Send-safe
    /// impl to hand to `Interp::set_host_fs`.
    pub fn register(obj: js_sys::Object) -> JsFs {
        let id = next_id();
        FS_OBJECTS.with(|m| m.borrow_mut().insert(id, obj));
        JsFs { id }
    }
}

impl Drop for JsFs {
    fn drop(&mut self) {
        FS_OBJECTS.with(|m| m.borrow_mut().remove(&self.id));
    }
}

fn fs_obj(id: u64) -> io::Result<js_sys::Object> {
    FS_OBJECTS.with(|m| {
        m.borrow()
            .get(&id)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "fs bridge dropped"))
    })
}

impl HostFs for JsFs {
    fn open(&mut self, path: &str, mode: &str) -> io::Result<(Box<dyn Channel>, bool)> {
        let obj = fs_obj(self.id)?;
        let binary = mode.to_ascii_lowercase().contains("binary")
            || mode.to_ascii_lowercase().contains('b');
        let handle = call_ret(&obj, "open", &[&JsValue::from_str(path), &JsValue::from_str(mode)])?;
        if !handle.is_object() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "fs.open did not return a file handle object",
            ));
        }
        let fid = next_id();
        FILE_OBJECTS.with(|m| {
            m.borrow_mut()
                .insert(fid, handle.dyn_into::<js_sys::Object>().unwrap())
        });
        Ok((Box::new(JsChannel { fid }), binary))
    }

    fn exists(&self, path: &str) -> bool {
        let Ok(obj) = fs_obj(self.id) else { return false };
        matches!(
            call_ret(&obj, "stat", &[&JsValue::from_str(path)]),
            Ok(v) if v.is_object()
        )
    }
    fn is_file(&self, path: &str) -> bool {
        let Ok(obj) = fs_obj(self.id) else { return false };
        matches!(
            call_ret(&obj, "stat", &[&JsValue::from_str(path)]),
            Ok(v) if v.is_object()
                && js_sys::Reflect::get(&v, &JsValue::from_str("file"))
                    .map(|b| b.is_truthy())
                    .unwrap_or(false)
        )
    }
    fn is_dir(&self, path: &str) -> bool {
        let Ok(obj) = fs_obj(self.id) else { return false };
        matches!(
            call_ret(&obj, "stat", &[&JsValue::from_str(path)]),
            Ok(v) if v.is_object()
                && js_sys::Reflect::get(&v, &JsValue::from_str("dir"))
                    .map(|b| b.is_truthy())
                    .unwrap_or(false)
        )
    }

    fn size(&self, path: &str) -> io::Result<u64> {
        let obj = fs_obj(self.id)?;
        let v = call_ret(&obj, "stat", &[&JsValue::from_str(path)])?;
        if !v.is_object() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such file or directory"));
        }
        let n = js_sys::Reflect::get(&v, &JsValue::from_str("size"))
            .map_err(|e| js_err(&e))?;
        Ok(n.as_f64().unwrap_or(0.0) as u64)
    }

    fn mtime(&self, path: &str) -> io::Result<i64> {
        let obj = fs_obj(self.id)?;
        let v = call_ret(&obj, "stat", &[&JsValue::from_str(path)])?;
        if !v.is_object() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such file or directory"));
        }
        let n = js_sys::Reflect::get(&v, &JsValue::from_str("mtime")).map_err(|e| js_err(&e))?;
        Ok(n.as_f64().unwrap_or(0.0) as i64)
    }

    fn atime(&self, path: &str) -> io::Result<i64> {
        self.mtime(path)
    }

    fn readable(&self, path: &str) -> bool {
        let Ok(obj) = fs_obj(self.id) else { return false };
        matches!(
            call_ret(&obj, "access", &[&JsValue::from_str(path)]),
            Ok(v) if v.is_object()
                && js_sys::Reflect::get(&v, &JsValue::from_str("read"))
                    .map(|b| b.is_truthy())
                    .unwrap_or(false)
        )
    }
    fn writable(&self, path: &str) -> bool {
        let Ok(obj) = fs_obj(self.id) else { return false };
        matches!(
            call_ret(&obj, "access", &[&JsValue::from_str(path)]),
            Ok(v) if v.is_object()
                && js_sys::Reflect::get(&v, &JsValue::from_str("write"))
                    .map(|b| b.is_truthy())
                    .unwrap_or(false)
        )
    }
    fn executable(&self, _path: &str) -> bool {
        false
    }

    fn mkdir(&mut self, path: &str, all: bool) -> io::Result<()> {
        let obj = fs_obj(self.id)?;
        call_unit(&obj, "mkdir", &[&JsValue::from_str(path), &JsValue::from_bool(all)])
    }
    fn delete(&mut self, path: &str) -> io::Result<()> {
        let obj = fs_obj(self.id)?;
        call_unit(&obj, "delete", &[&JsValue::from_str(path)])
    }
    fn rename(&mut self, src: &str, dst: &str) -> io::Result<()> {
        let obj = fs_obj(self.id)?;
        call_unit(&obj, "rename", &[&JsValue::from_str(src), &JsValue::from_str(dst)])
    }
    fn copy(&mut self, src: &str, dst: &str) -> io::Result<()> {
        let obj = fs_obj(self.id)?;
        call_unit(&obj, "copy", &[&JsValue::from_str(src), &JsValue::from_str(dst)])
    }
    fn list_dir(&self, path: &str) -> io::Result<Vec<String>> {
        let obj = fs_obj(self.id)?;
        let v = call_ret(&obj, "readdir", &[&JsValue::from_str(path)])?;
        let arr = js_sys::Array::try_from(v).map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "fs.readdir did not return an array")
        })?;
        let mut out = Vec::new();
        for i in 0..arr.length() {
            if let Some(s) = arr.get(i).as_string() {
                out.push(s);
            }
        }
        Ok(out)
    }
    fn pwd(&self) -> String {
        let Ok(obj) = fs_obj(self.id) else { return "/".into() };
        call_ret(&obj, "pwd", &[])
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| "/".to_string())
    }
    fn cd(&mut self, path: &str) -> io::Result<()> {
        let obj = fs_obj(self.id)?;
        call_unit(&obj, "cd", &[&JsValue::from_str(path)])
    }
    fn read_file(&mut self, path: &str) -> io::Result<String> {
        let obj = fs_obj(self.id)?;
        call_ret(&obj, "readfile", &[&JsValue::from_str(path)])?
            .as_string()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "fs.readfile did not return a string"))
    }
    fn tempfile(&mut self, prefix: &str) -> io::Result<String> {
        let obj = fs_obj(self.id)?;
        call_ret(&obj, "tempfile", &[&JsValue::from_str(prefix)])?
            .as_string()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "fs.tempfile did not return a string"))
    }

    fn stat_full(&self, path: &str, follow: bool) -> io::Result<rtcl_vm::host::FsStat> {
        let obj = fs_obj(self.id)?;
        let follow_v = JsValue::from_bool(follow);
        let v = call_ret(&obj, "statfull", &[&JsValue::from_str(path), &follow_v])
            .or_else(|_| {
                // 契约的最小形：宿主只给 stat(path) 时由基础面合成。
                call_ret(&obj, "stat", &[&JsValue::from_str(path)])
            })?;
        if !v.is_object() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such file or directory"));
        }
        let g = |k: &str| -> i64 {
            js_sys::Reflect::get(&v, &JsValue::from_str(k))
                .ok()
                .and_then(|n| n.as_f64())
                .unwrap_or(0.0) as i64
        };
        let truthy = |k: &str| -> bool {
            js_sys::Reflect::get(&v, &JsValue::from_str(k))
                .map(|b| b.is_truthy())
                .unwrap_or(false)
        };
        let size = g("size");
        let mtime = g("mtime");
        let atime = g("atime");
        let is_dir = truthy("dir");
        let is_file = truthy("file");
        let is_link = truthy("link");
        Ok(rtcl_vm::host::FsStat {
            size,
            mtime,
            atime,
            ctime: g("ctime"),
            dev: g("dev"),
            ino: g("ino"),
            nlink: g("nlink"),
            uid: g("uid"),
            gid: g("gid"),
            mode: g("mode"),
            blksize: g("blksize"),
            blocks: g("blocks"),
            is_dir,
            is_file,
            is_link,
            readable: true,
            writable: true,
            executable: false,
            owned: true,
        })
    }

    fn set_times(
        &mut self,
        path: &str,
        atime: Option<i64>,
        mtime: Option<i64>,
    ) -> io::Result<()> {
        let obj = fs_obj(self.id)?;
        let av = atime.map(|t| JsValue::from(t)).unwrap_or(JsValue::NULL);
        let mv = mtime.map(|t| JsValue::from(t)).unwrap_or(JsValue::NULL);
        call_unit(&obj, "settimes", &[&JsValue::from_str(path), &av, &mv])
    }

    fn read_link(&self, path: &str) -> io::Result<String> {
        let obj = fs_obj(self.id)?;
        call_ret(&obj, "readlink", &[&JsValue::from_str(path)])?
            .as_string()
            .ok_or_else(|| {
                io::Error::from_raw_os_error(22)
            })
    }
}

fn file_obj(fid: u64) -> io::Result<js_sys::Object> {
    FILE_OBJECTS.with(|m| {
        m.borrow()
            .get(&fid)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "file handle dropped"))
    })
}

/// JS-backed channel: the handle object's methods ARE the data plane.
pub struct JsChannel {
    fid: u64,
}

impl Channel for JsChannel {
    fn read_bytes(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let obj = file_obj(self.fid)?;
        let v = call_ret(&obj, "read", &[&JsValue::from(buf.len() as u32)])?;
        let Some(s) = v.as_string() else { return Ok(0) };
        if s.is_empty() {
            return Ok(0);
        }
        let n = buf.len().min(s.chars().count());
        for (i, c) in s.chars().take(n).enumerate() {
            buf[i] = (c as u32) as u8;
        }
        Ok(n)
    }

    fn read_line(&mut self) -> io::Result<Option<String>> {
        let obj = file_obj(self.fid)?;
        let v = call_ret(&obj, "readline", &[])?;
        if v.is_null() || v.is_undefined() {
            return Ok(None);
        }
        Ok(v.as_string())
    }

    fn read_all(&mut self) -> io::Result<String> {
        let obj = file_obj(self.fid)?;
        call_ret(&obj, "readall", &[])?
            .as_string()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "readall did not return a string"))
    }

    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> {
        let obj = file_obj(self.fid)?;
        let s: String = data.iter().map(|&b| b as char).collect();
        call_unit(&obj, "write", &[&JsValue::from_str(&s)])?;
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let obj = file_obj(self.fid)?;
        call_unit(&obj, "flush", &[])
    }

    fn seek(&mut self, whence: io::SeekFrom) -> io::Result<u64> {
        let obj = file_obj(self.fid)?;
        let (wh, pos) = match whence {
            io::SeekFrom::Start(p) => ("set", p as f64),
            io::SeekFrom::End(o) => ("end", o as f64),
            io::SeekFrom::Current(o) => ("cur", o as f64),
        };
        let v = call_ret(&obj, "seek", &[&JsValue::from(pos), &JsValue::from_str(wh)])?;
        Ok(v.as_f64().unwrap_or(0.0) as u64)
    }

    fn tell(&mut self) -> io::Result<u64> {
        let obj = file_obj(self.fid)?;
        let v = call_ret(&obj, "tell", &[])?;
        Ok(v.as_f64().unwrap_or(0.0) as u64)
    }

    fn eof(&self) -> bool {
        let Ok(obj) = file_obj(self.fid) else { return true };
        matches!(
            call_ret(&obj, "eof", &[]),
            Ok(v) if v.is_truthy()
        )
    }

    fn close(self: Box<Self>) -> io::Result<()> {
        let obj = file_obj(self.fid)?;
        call_unit(&obj, "close", &[])?;
        FILE_OBJECTS.with(|m| m.borrow_mut().remove(&self.fid));
        Ok(())
    }

    fn is_readable(&self) -> bool {
        true
    }
    fn is_writable(&self) -> bool {
        true
    }
    fn channel_type(&self) -> &'static str {
        "jsfs"
    }
}
