#!/usr/bin/env bash
# pin tclsh socket oracle details:
#  1. raw full-query rendering/order of fconfigure on a socket + peername shape
#  1b. write-side translation: does \n leave as CRLF? (binary echo round-trip)
#  2. socket -async to live server: immediate handle, later I/O works
#  3. async connect failure surface text (dead port)
#  4. sync connect failure text (dead port)
set -u
cd "$(dirname "$0")/../.."

SRV_LINE=$(mktemp /tmp/sa_srvline.XXXX)
SRV_ECHO=$(mktemp /tmp/sa_srvecho.XXXX)
CLI=$(mktemp /tmp/sa_cli.XXXX)
OUT=$(mktemp /tmp/sa_out.XXXX)

cat > "$SRV_LINE" <<'EOF'
set port [lindex $argv 0]
proc accept {ch addr hostport} { puts $ch "hi"; close $ch }
socket -server accept $port
vwait forever
EOF

cat > "$SRV_ECHO" <<'EOF'
set port [lindex $argv 0]
proc accept {ch addr hostport} {
    fconfigure $ch -translation binary
    set d [read $ch 4]
    puts -nonewline $ch $d
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

port=$(pick_port)
port_echo=$(pick_port)
port_dead=$(pick_port)
tclsh "$SRV_LINE" "$port" >/dev/null 2>&1 & P1=$!
tclsh "$SRV_ECHO" "$port_echo" >/dev/null 2>&1 & P2=$!
sleep 0.3

cat > "$CLI" <<EOF
# 1. raw full query on a socket (value rendering incl. braces)
set s [socket 127.0.0.1 $port]
set q [fconfigure \$s]
puts "rawq=<\$q>"
set p [fconfigure \$s -peername]
puts "peer0=[lindex \$p 0] peer1=[lindex \$p 1]"
close \$s
# 1b. write-side translation: does \\n leave as CRLF? (binary echo)
set e [socket 127.0.0.1 $port_echo]
fconfigure \$e -translation binary
puts -nonewline \$e "AB\nC"
flush \$e
set back [read \$e 4]
binary scan \$back H* hex
puts "echo-hex=\$hex"
close \$e
# 1c. default translation on a socket, binary-safe write via crlf?
set e2 [socket 127.0.0.1 $port_echo]
puts "e2-trans=[fconfigure \$e2 -translation]"
close \$e2
# 2. async connect to a live server: immediate handle, then works
set a [socket -async 127.0.0.1 $port]
puts "async-handle=[string match sock* \$a] blocking=[fconfigure \$a -blocking]"
set n [catch {flush \$a} m1]
puts "flush-live|0|\$n"
puts "async-gets=[gets \$a]"
close \$a
# 3. async connect to a dead port: failure surface text
set d [socket -async 127.0.0.1 $port_dead]
set n [catch {puts \$d x} m2]
puts "puts-dead|\$n|\$m2"
set n [catch {flush \$d} m3]
puts "flush-dead|\$n|\$m3|\$::errorCode"
set n [catch {gets \$d} m4]
puts "gets-dead|\$n|\$m4|\$::errorCode"
set n [catch {read \$d} m5]
puts "read-dead|\$n|\$m5|\$::errorCode"
set n [catch {fconfigure \$d -peername} m6]
puts "peer-dead|\$n|\$m6|\$::errorCode"
set n [catch {close \$d} m7]
puts "close-dead|\$n|\$m7|\$::errorCode"
# 4. sync connect to a dead port (immediate error text)
set n [catch {socket 127.0.0.1 $port_dead} m8]
puts "sync-dead|\$n|\$m8|\$::errorCode"
exit 0
EOF

timeout 15 tclsh "$CLI" > "$OUT" 2>&1
echo "rc=$?"
cat "$OUT"
kill $P1 $P2 2>/dev/null
rm -f "$SRV_LINE" "$SRV_ECHO" "$CLI" "$OUT"
