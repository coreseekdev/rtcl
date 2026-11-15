#!/usr/bin/env bash
# socket 数据面差分（tclsh vs rtcl，tclsh 当 server）：
#  1. CRLF 落线：默认 translation 下 puts 的 \n 落线即 \r\n（binary
#     echo 回读 hex 比对）
#  2. ECONNRESET → EOF：server 收数据不读即 close（RST），client read
#     得 EOF（空串 + eof=1），不抛错
set -u
cd "$(dirname "$0")/../.."
RTCL=./target/release/rtcl

SRV_ECHO=$(mktemp /tmp/dp_echo.XXXX)
SRV_RST=$(mktemp /tmp/dp_rst.XXXX)
CLI=$(mktemp /tmp/dp_cli.XXXX)
O1=$(mktemp /tmp/dp_o1.XXXX)
O2=$(mktemp /tmp/dp_o2.XXXX)

cat > "$SRV_ECHO" <<'EOF'
set port [lindex $argv 0]
proc accept {ch addr hostport} {
    fconfigure $ch -translation binary -buffering none
    set d [read $ch 5]
    puts -nonewline $ch $d
    close $ch
}
socket -server accept $port
vwait forever
EOF

cat > "$SRV_RST" <<'EOF'
set port [lindex $argv 0]
proc accept {ch addr hostport} {
    # 不读即 close：client 已送出的未读数据 → RST
    close $ch
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

pe=$(pick_port); pr=$(pick_port)
tclsh "$SRV_ECHO" "$pe" >/dev/null 2>&1 & P1=$!
sleep 0.2
tclsh "$SRV_RST" "$pr" >/dev/null 2>&1 & P2=$!
sleep 0.2

cat > "$CLI" <<EOF
if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} {
  set c [catch {uplevel 1 \$script} r]
  regsub -all {sock[0-9a-f]+(?![a-z])} \$r sockX r
  set ec \$::errorCode
  regsub -all {sock[0-9a-f]+(?![a-z])} \$ec sockX ec
  puts "\$label|\$c|\$r|EC=\$ec"
}
t crlf-wire {set e [socket 127.0.0.1 $pe]; fconfigure \$e -translation {binary crlf}; puts -nonewline \$e "AB\nC"; flush \$e; set back [read \$e 5]; binary scan \$back H* hex; close \$e; set hex}
t reset-eof {set s [socket 127.0.0.1 $pr]; puts \$s hello; flush \$s; set r1 [read \$s]; set e1 [eof \$s]; close \$s; list r1=<\$r1> eof=\$e1}
t reset-read-again {set s [socket 127.0.0.1 $pr]; puts \$s x; flush \$s; set a [read \$s]; set b [catch {read \$s} m]; close \$s; list a=<\$a> second=\$b}
exit 0
EOF

timeout 20 "$RTCL" -f "$CLI" > "$O1" 2>&1; R1=$?
timeout 20 tclsh "$CLI" > "$O2" 2>&1; R2=$?
kill $P1 $P2 2>/dev/null; wait $P1 $P2 2>/dev/null

echo "--- rtcl(rc=$R1) ---"; cat "$O1"
echo "--- tclsh(rc=$R2) ---"; cat "$O2"
echo "--- diff ---"
diff "$O2" "$O1" && echo SAME || echo DIFF
rm -f "$SRV_ECHO" "$SRV_RST" "$CLI" "$O1" "$O2"
