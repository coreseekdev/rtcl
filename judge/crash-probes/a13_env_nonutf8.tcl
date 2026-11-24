# crash-probe: a13 env nonutf8
# gate: env-nonutf8
# gate: skip-tree
source [file join [file dirname [info script]] _chk.tcl]
# The gate starts this interpreter with a non-UTF-8 variable in its
# environment: `Interp::new()` used to panic on it (std::env::vars()).
chk constructed 1 1
chk bad-present [info exists env(RTCL_BAD)] 1
# ... and in a scrubbed environment, so nothing else leaked in.
chk scrubbed [info exists env(HOME)] 0
chk lookup [catch {env RTCL_BAD} e] 0
