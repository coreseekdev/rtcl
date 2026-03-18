//! Event loop commands: after, vwait, update.
//!
//! Implements the jimtcl event loop model:
//! - `after ms` — synchronous sleep
//! - `after ms script` — schedule a timed event
//! - `after idle script` — schedule an idle event (fires on next `update`/`vwait`)
//! - `after cancel id|script` — cancel a scheduled event
//! - `after info ?id?` — list or query scheduled events
//! - `vwait varName` — enter event loop until variable changes
//! - `update ?idletasks?` — process pending events

use crate::error::{Error, ErrorCode, Result};
use crate::interp::Interp;
use crate::value::Value;

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Process-wide epoch for monotonic time.
fn epoch() -> &'static Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now)
}

/// Current monotonic time in milliseconds since process start.
fn now_ms() -> u64 {
    epoch().elapsed().as_millis() as u64
}

// ---------- after ----------

/// `after ms ?script ...?`
pub fn cmd_after(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err(Error::wrong_args_with_usage(
            "after", 2, args.len(),
            "after option ?arg ...?",
        ));
    }

    let sub = args[1].as_str();

    match sub {
        "cancel" => after_cancel(interp, args),
        "info" => after_info(interp, args),
        "idle" => after_idle(interp, args),
        _ => {
            // Try to parse as integer milliseconds
            let ms: u64 = sub.parse().map_err(|_| {
                Error::runtime(
                    format!("bad argument \"{}\": must be cancel, idle, info, or an integer", sub),
                    ErrorCode::Generic,
                )
            })?;

            if args.len() == 2 {
                // after ms — synchronous sleep
                std::thread::sleep(Duration::from_millis(ms));
                Ok(Value::empty())
            } else {
                // after ms script ?script ...? — schedule timed event
                let script = if args.len() == 3 {
                    args[2].as_str().to_string()
                } else {
                    // Concatenate remaining args
                    args[2..].iter()
                        .map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                let id = schedule_event(interp, ms, script, false);
                Ok(Value::from_str(&format!("after#{}", id)))
            }
        }
    }
}

fn schedule_event(interp: &mut Interp, delay_ms: u64, script: String, is_idle: bool) -> u64 {
    let id = interp.next_event_id;
    interp.next_event_id += 1;
    let fire_at_ms = if is_idle { 0 } else { now_ms() + delay_ms };
    interp.event_queue.push(crate::interp::TimedEvent {
        id,
        fire_at_ms,
        script,
        is_idle,
    });
    id
}

fn after_idle(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "after", 3, args.len(),
            "after idle script ?script ...?",
        ));
    }
    let script = if args.len() == 3 {
        args[2].as_str().to_string()
    } else {
        args[2..].iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let id = schedule_event(interp, 0, script, true);
    Ok(Value::from_str(&format!("after#{}", id)))
}

fn after_cancel(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err(Error::wrong_args_with_usage(
            "after", 3, args.len(),
            "after cancel id|command",
        ));
    }
    let target = args[2].as_str();

    // Try matching as "after#N"
    if let Some(id_str) = target.strip_prefix("after#") {
        if let Ok(id) = id_str.parse::<u64>() {
            interp.event_queue.retain(|ev| ev.id != id);
            return Ok(Value::empty());
        }
    }

    // Otherwise match by script text (concatenate remaining args if needed)
    let script = if args.len() == 3 {
        target.to_string()
    } else {
        args[2..].iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    interp.event_queue.retain(|ev| ev.script != script);
    Ok(Value::empty())
}

fn after_info(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() > 3 {
        return Err(Error::wrong_args_with_usage(
            "after", 2, args.len(),
            "after info ?id?",
        ));
    }

    if args.len() == 2 {
        // List all pending event IDs
        let ids: Vec<String> = interp.event_queue.iter()
            .map(|ev| format!("after#{}", ev.id))
            .collect();
        return Ok(Value::from_str(&ids.join(" ")));
    }

    // Query specific event
    let target = args[2].as_str();
    let id = target.strip_prefix("after#")
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| Error::runtime(
            format!("event \"{}\" doesn't exist", target),
            ErrorCode::NotFound,
        ))?;

    let ev = interp.event_queue.iter()
        .find(|ev| ev.id == id)
        .ok_or_else(|| Error::runtime(
            format!("event \"{}\" doesn't exist", target),
            ErrorCode::NotFound,
        ))?;

    let kind = if ev.is_idle { "idle" } else { "timer" };
    Ok(Value::from_str(&format!("{} {}", ev.script, kind)))
}

// ---------- vwait ----------

/// `vwait varName`
///
/// Enter the event loop and process events until the named variable is
/// modified (set or unset).
pub fn cmd_vwait(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    if args.len() != 2 {
        return Err(Error::wrong_args_with_usage(
            "vwait", 2, args.len(),
            "vwait varName",
        ));
    }
    let varname = args[1].as_str();

    // Capture the current value of the variable
    let initial = interp.get_var(varname).ok().map(|v| v.as_str().to_string());

    loop {
        // Process at least one round of events
        let processed = process_events(interp, false)?;

        // Check if variable has changed
        let current = interp.get_var(varname).ok().map(|v| v.as_str().to_string());
        if current != initial {
            break;
        }

        // If no events were processed and variable hasn't changed, sleep briefly to avoid busy-wait
        if processed == 0 {
            if interp.event_queue.is_empty() {
                // No events pending at all — variable can only change if we break out
                return Err(Error::runtime(
                    format!("can't wait for variable \"{}\": would wait forever", varname),
                    ErrorCode::Generic,
                ));
            }
            // Sleep until the next timed event
            let next_fire = interp.event_queue.iter()
                .filter(|ev| !ev.is_idle)
                .map(|ev| ev.fire_at_ms)
                .min();
            if let Some(fire_at) = next_fire {
                let now = now_ms();
                if fire_at > now {
                    std::thread::sleep(Duration::from_millis(fire_at - now));
                }
            } else {
                // Only idle events — process them
                process_events(interp, true)?;
            }
        }
    }

    Ok(Value::empty())
}

// ---------- update ----------

/// `update ?idletasks?`
///
/// Process pending events non-blocking.
pub fn cmd_update(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    let idle_only = if args.len() >= 2 {
        let sub = args[1].as_str();
        if sub == "idletasks" {
            true
        } else {
            return Err(Error::runtime(
                format!("bad option \"{}\": must be idletasks", sub),
                ErrorCode::Generic,
            ));
        }
    } else {
        false
    };

    // Process all currently-ready events (non-blocking)
    loop {
        let processed = process_events(interp, idle_only)?;
        if processed == 0 {
            break;
        }
    }

    Ok(Value::empty())
}

// ---------- Event processing ----------

/// Process ready events from the queue. Returns the number of events fired.
///
/// If `idle_only` is true, only idle events are processed.
fn process_events(interp: &mut Interp, idle_only: bool) -> Result<usize> {
    let now = now_ms();
    let mut fired = 0;

    // Collect indices of ready events (fire in order of scheduling)
    loop {
        // Find the next ready event
        let idx = interp.event_queue.iter().position(|ev| {
            if idle_only {
                ev.is_idle
            } else {
                ev.is_idle || ev.fire_at_ms <= now
            }
        });

        let Some(idx) = idx else { break };

        // Remove the event before executing (prevents infinite loops with `after 0`)
        let ev = interp.event_queue.remove(idx);
        fired += 1;

        // Execute the script, ignoring errors (like jimtcl's bgerror handling)
        let _ = interp.eval(&ev.script);
    }

    Ok(fired)
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use crate::interp::Interp;

    #[test]
    fn test_after_sleep() {
        let mut interp = Interp::new();
        // after 0 should return immediately (sleep 0ms)
        let r = interp.eval("after 0").unwrap();
        assert_eq!(r.as_str(), "");
    }

    #[test]
    fn test_after_schedule_and_info() {
        let mut interp = Interp::new();
        let id = interp.eval("after 100000 {set x 1}").unwrap();
        assert!(id.as_str().starts_with("after#"));

        // after info should list the event
        let info = interp.eval("after info").unwrap();
        assert!(info.as_str().contains(id.as_str()));

        // after info <id> should return script + type
        let detail = interp.eval(&format!("after info {}", id.as_str())).unwrap();
        assert!(detail.as_str().contains("set x 1"));
        assert!(detail.as_str().contains("timer"));
    }

    #[test]
    fn test_after_cancel_by_id() {
        let mut interp = Interp::new();
        let id = interp.eval("after 100000 {set x 1}").unwrap();
        interp.eval(&format!("after cancel {}", id.as_str())).unwrap();
        let info = interp.eval("after info").unwrap();
        assert_eq!(info.as_str(), "");
    }

    #[test]
    fn test_after_cancel_by_script() {
        let mut interp = Interp::new();
        interp.eval("after 100000 {set x 1}").unwrap();
        interp.eval("after cancel {set x 1}").unwrap();
        let info = interp.eval("after info").unwrap();
        assert_eq!(info.as_str(), "");
    }

    #[test]
    fn test_after_idle() {
        let mut interp = Interp::new();
        let id = interp.eval("after idle {set x 42}").unwrap();
        assert!(id.as_str().starts_with("after#"));

        let detail = interp.eval(&format!("after info {}", id.as_str())).unwrap();
        assert!(detail.as_str().contains("idle"));

        // update should fire idle events
        interp.eval("update").unwrap();
        let x = interp.eval("set x").unwrap();
        assert_eq!(x.as_str(), "42");
    }

    #[test]
    fn test_after_timed_fires_on_update() {
        let mut interp = Interp::new();
        // Schedule event with 0ms delay (fires immediately)
        interp.eval("after 0 {set y hello}").unwrap();
        // Small sleep to ensure time passes
        std::thread::sleep(std::time::Duration::from_millis(10));
        interp.eval("update").unwrap();
        let y = interp.eval("set y").unwrap();
        assert_eq!(y.as_str(), "hello");
    }

    #[test]
    fn test_update_idletasks() {
        let mut interp = Interp::new();
        // Schedule a timed event and an idle event
        interp.eval("after 0 {set a timer_fired}").unwrap();
        interp.eval("after idle {set b idle_fired}").unwrap();

        // update idletasks should only fire idle events
        interp.eval("update idletasks").unwrap();

        // Idle event should have fired
        let b = interp.eval("set b").unwrap();
        assert_eq!(b.as_str(), "idle_fired");

        // Timer event should still be pending (idletasks doesn't process timers)
        assert!(interp.eval("set a").is_err());
    }

    #[test]
    fn test_vwait_with_after() {
        let mut interp = Interp::new();
        // Schedule setting the watched variable after 0ms
        interp.eval("after 0 {set done 1}").unwrap();
        interp.eval("vwait done").unwrap();
        let done = interp.eval("set done").unwrap();
        assert_eq!(done.as_str(), "1");
    }

    #[test]
    fn test_after_multiple_events_order() {
        let mut interp = Interp::new();
        interp.eval("set result {}").unwrap();
        interp.eval("after 0 {append result a}").unwrap();
        interp.eval("after 0 {append result b}").unwrap();
        interp.eval("after 0 {append result c}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        interp.eval("update").unwrap();
        let r = interp.eval("set result").unwrap();
        assert_eq!(r.as_str(), "abc");
    }
}
