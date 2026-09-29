//! A write-only channel that forwards `puts` output to a JavaScript callback.

use std::io;
use wasm_bindgen::prelude::*;
use rtcl_core::channel::Channel;
use crate::js_command::js_error_message;

/// Write-only stdout replacement backed by a JS callback.
///
/// Bytes are buffered and converted to a string on flush, so a `puts`
/// invocation reaches the callback as one complete UTF-8 chunk.
///
/// # Safety
/// `Send` is required by `Channel`, but `js_sys::Function` is not `Send`.
/// WASM runs single-threaded and the handle is only used from the JS thread,
/// so sharing the callback across a `Send` bound is sound in practice.
pub struct JsOutputChannel {
    callback: js_sys::Function,
    buf: Vec<u8>,
}

// SAFETY: single-threaded WASM — see doc comment above.
unsafe impl Send for JsOutputChannel {}

impl JsOutputChannel {
    pub fn new(callback: js_sys::Function) -> Self {
        JsOutputChannel { callback, buf: Vec::new() }
    }

    fn emit(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let s = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        self.callback
            .call1(&JsValue::NULL, &JsValue::from_str(&s))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, js_error_message(&e)))?;
        Ok(())
    }
}

impl Channel for JsOutputChannel {
    fn read_bytes(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdout\" wasn't opened for reading"))
    }

    fn read_line(&mut self) -> io::Result<Option<String>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdout\" wasn't opened for reading"))
    }

    fn read_all(&mut self) -> io::Result<String> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "channel \"stdout\" wasn't opened for reading"))
    }

    fn write_bytes(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.emit()
    }

    fn seek(&mut self, _whence: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stdout is not seekable"))
    }

    fn tell(&mut self) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "stdout is not seekable"))
    }

    fn eof(&self) -> bool {
        false
    }

    fn close(self: Box<Self>) -> io::Result<()> {
        // Flush any buffered bytes before the channel goes away.
        let mut this = *self;
        this.emit()?;
        Ok(())
    }

    fn is_readable(&self) -> bool {
        false
    }

    fn is_writable(&self) -> bool {
        true
    }

    fn channel_type(&self) -> &'static str {
        "js-output"
    }
}
