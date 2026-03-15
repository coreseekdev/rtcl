//! rtcl-expect - Expect-style process automation
//!
//! This crate provides expect-like functionality for rtcl.
//! Including process spawning, interaction, and pseudo-terminal handling.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Error type for expect module
#[derive(Debug)]
pub enum ExpectError {
    /// Failed to spawn process
    SpawnFailed(String),
    /// Process exited unexpectedly
    ProcessExit(String),
    /// Timeout waiting for pattern
    Timeout(String),
    /// Pattern not found
    PatternNotFound(String),
    /// IO error
    Io(String),
}

impl std::fmt::Display for ExpectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExpectError::SpawnFailed(msg) => write!(f, "spawn failed: {}", msg),
            ExpectError::ProcessExit(msg) => write!(f, "process exited: {}", msg),
            ExpectError::Timeout(msg) => write!(f, "timeout: {}", msg),
            ExpectError::PatternNotFound(msg) => write!(f, "pattern not found: {}", msg),
            ExpectError::Io(msg) => write!(f, "io error: {}", msg),
        }
    }
}

impl std::error::Error for ExpectError {}

/// A spawned process for expect interactions
pub struct SpawnedProcess {
    /// The child process
    child: Child,
    /// Buffer for output matching
    buffer: String,
}

impl SpawnedProcess {
    /// Spawn a new process
    pub fn spawn(program: &str, args: &[&str]) -> Result<Self, ExpectError> {
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ExpectError::SpawnFailed(format!("{}: {}", program, e)))?;

        Ok(SpawnedProcess {
            child,
            buffer: String::new(),
        })
    }

    /// Get the process ID
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Check if process is still running
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Send a string to the process
    pub fn send(&mut self, data: &str) -> Result<(), ExpectError> {
        let stdin = self.child.stdin.as_mut()
            .ok_or_else(|| ExpectError::Io("stdin not available".to_string()))?;

        stdin.write_all(data.as_bytes())
            .map_err(|e| ExpectError::Io(e.to_string()))?;
        stdin.flush()
            .map_err(|e| ExpectError::Io(e.to_string()))?;

        Ok(())
    }

    /// Send a line to the process (with newline)
    pub fn send_line(&mut self, line: &str) -> Result<(), ExpectError> {
        self.send(&format!("{}\n", line))
    }

    /// Read available output (non-blocking)
    pub fn read_available(&mut self) -> Result<String, ExpectError> {
        let stdout = self.child.stdout.as_mut()
            .ok_or_else(|| ExpectError::Io("stdout not available".to_string()))?;

        // Set non-blocking (platform specific)
        let mut buf = [0u8; 4096];
        let mut result = String::new();

        // Try to read available data
        match stdout.read(&mut buf) {
            Ok(0) => { /* EOF */ }
            Ok(n) => {
                result.push_str(&String::from_utf8_lossy(&buf[..n]));
                self.buffer.push_str(&result);
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    return Err(ExpectError::Io(e.to_string()));
                }
            }
        }

        Ok(result)
    }

    /// Wait for a pattern in the output
    pub fn expect(&mut self, pattern: &str, timeout: Duration) -> Result<String, ExpectError> {
        let start = std::time::Instant::now();

        loop {
            // Check if pattern is already in buffer
            if let Some(pos) = self.buffer.find(pattern) {
                let matched = self.buffer[..pos + pattern.len()].to_string();
                self.buffer = self.buffer[pos + pattern.len()..].to_string();
                return Ok(matched);
            }

            // Check timeout
            if start.elapsed() > timeout {
                return Err(ExpectError::Timeout(format!(
                    "pattern '{}' not found within {:?}",
                    pattern, timeout
                )));
            }

            // Read more data
            self.read_available()?;

            // Small sleep to avoid busy-waiting
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Wait for the process to exit
    pub fn wait(&mut self) -> Result<std::process::ExitStatus, ExpectError> {
        self.child.wait()
            .map_err(|e| ExpectError::Io(e.to_string()))
    }

    /// Kill the process
    pub fn kill(&mut self) -> Result<(), ExpectError> {
        self.child.kill()
            .map_err(|e| ExpectError::Io(e.to_string()))
    }

    /// Get the current buffer contents
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Clear the buffer
    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
    }
}

impl Drop for SpawnedProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Convenience function to spawn and interact with a process
pub fn spawn(program: &str, args: &[&str]) -> Result<SpawnedProcess, ExpectError> {
    SpawnedProcess::spawn(program, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn test_spawn_echo() {
        let mut proc = spawn("echo", &["hello"]).expect("Failed to spawn");
        let result = proc.expect("hello", Duration::from_secs(5));
        assert!(result.is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn test_spawn_echo() {
        let mut proc = spawn("cmd", &["/c", "echo hello"]).expect("Failed to spawn");
        let result = proc.expect("hello", Duration::from_secs(5));
        assert!(result.is_ok());
    }
}
