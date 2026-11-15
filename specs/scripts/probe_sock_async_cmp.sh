#!/usr/bin/env bash
# socket -async 全景差分（tclsh vs rtcl）：live/dead 回环 + fconfigure
# socket 查询面。tclsh 当 server（rtcl 无 -server），两解释器当 client
# 跑同一生成脚本；端口每轮重新分配（rtcl -f 无 argv，端口烤进脚本）。
# 输出归一化：通道指针名/sockN → sockX（引号内匹配，避开 "socket" 词），
# 端口只打印整数类（两者端口必不同）。
set -u
cd "$(dirname "$0")/../.."
RTCL=./target/release/rtcl

SRV=$(mktemp /tmp/acmp_srv.XXXX)
CLI=$(mktemp /tmp/acmp_cli.XXXX)
O1=$(mktemp /tmp/acmp_o1.XXXX)
O2=$(mktemp /tmp/acmp_o2.XXXX)

cat > "$SRV" <<'EOF'
set port [lindex $argv 0]
proc accept {ch addr hostport} { puts $ch "hi"; close $ch }
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
dead=$(pick_port)
tclsh "$SRV" "$port" >/dev/null 2>&1 & SRV_PID=$!
sleep 0.3

cat > "$CLI" <<EOF
if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} {
  set c [catch {uplevel 1 \$script} r]
  # 归一化：通道指针名/sockN → sockX（引号内外都要；(?![a-z]) 防止
  # 吃掉 "socket" 一词——'e' 也是 hex 字符）。
  regsub -all {sock[0-9a-f]+(?![a-z])} \$r sockX r
  set ec \$::errorCode
  regsub -all {sock[0-9a-f]+(?![a-z])} \$ec sockX ec
  puts "\$label|\$c|\$r|EC=\$ec"
}
# ── live：async 连回环 echo server ──
t async-handle {set a [socket -async 127.0.0.1 $port]; string match sock* \$a}
t async-trans [list fconfigure \$a -translation]
t connecting-live {fconfigure \$a -connecting}
t flush-live {flush \$a}
t async-gets {gets \$a}
t connecting-after {fconfigure \$a -connecting}
t peer-live {lrange [fconfigure \$a -peername] 0 1}
t sockname-live {lrange [fconfigure \$a -sockname] 0 1}
t opts-names {set q [fconfigure \$a]; set ns {}; foreach {k v} \$q {lappend ns \$k}; lsort \$ns}
close \$a
# ── async 写出经 crlf 落线（binary echo 回读）──
# （server 是 line-server，无 echo；改连 sync 通道验证 crlf 面见
# probe_sock_peer.sh；此处只验 puts+flush 到 live server 不炸）
t async-write {set b [socket -async 127.0.0.1 $port]; puts \$b yo; flush \$b; close \$b; list ok}
# ── dead：async 连无监听端口（回环 RST）──
t async-dead {set d [socket -async 127.0.0.1 $dead]; string match sock* \$d}
t connecting-dead {fconfigure \$d -connecting}
t puts-dead {puts \$d x}
t flush-dead {flush \$d}
t gets-dead {gets \$d}
t read-dead {read \$d}
t peer-dead {fconfigure \$d -peername}
t sockname-dead {llength [fconfigure \$d -sockname]}
t sockname-e0 {lindex [fconfigure \$d -sockname] 0}
t close-dead {close \$d}
# ── sync 连死端口 ──
t sync-dead {socket 127.0.0.1 $dead}
# ── 选项表：socket 的 bad-option 单查询文案 ──
t bad-opt {set e2 [socket -async 127.0.0.1 $dead]; fconfigure \$e2 -nosuch}
t set-peer {fconfigure \$e2 -peername x}
t bad-prefix {fconfigure \$e2 -peer}
close \$e2
exit 0
EOF

timeout 20 "$RTCL" -f "$CLI" > "$O1" 2>&1; R1=$?
timeout 20 tclsh "$CLI" > "$O2" 2>&1; R2=$?
kill $SRV_PID 2>/dev/null; wait $SRV_PID 2>/dev/null

echo "--- rtcl(rc=$R1) ---"; cat "$O1"
echo "--- tclsh(rc=$R2) ---"; cat "$O2"
echo "--- diff ---"
diff "$O2" "$O1" && echo SAME || echo DIFF
rm -f "$SRV" "$CLI" "$O1" "$O2"
