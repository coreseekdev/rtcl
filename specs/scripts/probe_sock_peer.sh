#!/usr/bin/env bash
# socket fconfigure differential: tclsh vs rtcl as loopback CLIENTS against
# a tclsh echo server (rtcl has no -server). Port is baked into the
# generated script (rtcl -f has no argv pass-through). Deterministic:
# ports are printed as integer-class only; host fields come from
# 127.0.0.1 which resolves identically in both interpreters.
set -u
cd "$(dirname "$0")/../.."
RTCL=./target/release/rtcl

ORACLE_OUT=$(mktemp /tmp/sock_oracle.XXXX)
RTCL_OUT=$(mktemp /tmp/sock_rtcl.XXXX)
SRV=$(mktemp /tmp/sock_srv.XXXX)
CLI=$(mktemp /tmp/sock_cli.XXXX)

# The listener (tclsh only): accept one connection, write a line, close ->
# client sees EOF after buffered data.
cat > "$SRV" <<'EOF'
set port [lindex $argv 0]
proc accept {ch addr hostport} {
    puts $ch "hi"
    close $ch
    set ::done 1
}
socket -server accept $port
vwait forever
EOF

pick_port() { python3 - <<'PY'
import socket
s=socket.socket(); s.bind(("127.0.0.1",0))
print(s.getsockname()[1]); s.close()
PY
}

run_case() {
  # $1 = interpreter invocation prefix (tclsh takes the bare script path,
  # rtcl needs -f); $2 = output file
  local -a cmd=("$@")
  local out="${cmd[-1]}"
  unset 'cmd[-1]'
  local port; port=$(pick_port)
  tclsh "$SRV" "$port" >/dev/null 2>&1 & local srv=$!
  sleep 0.3
  # bake the port into the script so both interpreters share one text
  cat > "$CLI" <<EOF
if {![info exists ::errorCode]} {set ::errorCode NONE}
set s [socket 127.0.0.1 $port]
proc t {label script} {
  set c [catch {uplevel 1 \$script} r]
  regsub -all {sock[0-9a-f]+(?![a-z])} \$r sockX r
  set ec \$::errorCode
  regsub -all {sock[0-9a-f]+(?![a-z])} \$ec sockX ec
  puts "\$label|\$c|\$r|EC=\$ec"
}
set q [fconfigure \$s]
set names {}
foreach {k v} \$q {lappend names \$k}
puts "opts=[lsort \$names]"
set p [fconfigure \$s -peername]
puts "peer-len=[llength \$p] e0=[lindex \$p 0] e1=[lindex \$p 1] port-int=[string is integer -strict [lindex \$p 2]]"
set k [fconfigure \$s -sockname]
puts "sock-len=[llength \$k] e0=[lindex \$k 0] e1=[lindex \$k 1] port-int=[string is integer -strict [lindex \$k 2]]"
t gets1 {gets \$s}
t eof1 {eof \$s}
t read2 {read \$s}
t eof2 {eof \$s}
t flush-ok {flush \$s}
t puts-after-eof {puts \$s x; flush \$s}
t close {close \$s}
t gets-closed {gets \$s}
exit 0
EOF
  "${cmd[@]}" "$CLI" > "$out" 2>&1
  kill "$srv" 2>/dev/null; wait "$srv" 2>/dev/null
}

run_case tclsh "$ORACLE_OUT"
run_case "$RTCL" -f "$RTCL_OUT"
echo "--- oracle ---"; cat "$ORACLE_OUT"
echo "--- rtcl ---"; cat "$RTCL_OUT"
echo "--- diff ---"
diff "$ORACLE_OUT" "$RTCL_OUT" && echo SAME || echo DIFF
rm -f "$ORACLE_OUT" "$RTCL_OUT" "$SRV" "$CLI"
