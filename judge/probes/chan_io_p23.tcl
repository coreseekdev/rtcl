# socket 客户端面（self-contained，无 server——rtcl 无 socket -server，
# 伺服面差异见 DIVERGENCES）。只打死端口（0/1——回环上必拒），行为
# 全确定；通道名 sockN/指针名经归一化（两种命名模型不可复现）。
if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} {
  set c [catch {uplevel 1 $script} r]
  # 归一化：通道名（sockN/指针名）与回环临时端口（两者必不同）。
  regsub -all {sock[0-9a-f]+(?![a-z])} $r sockX r
  regsub -all {localhost [0-9]+} $r {localhost PORT} r
  set ec $::errorCode
  regsub -all {sock[0-9a-f]+(?![a-z])} $ec sockX ec
  puts "$label|$c|$r|EC=$ec"
}
# ── 端口参数解析（TclSockGetPort 形）──
t port-bad {socket 127.0.0.1 zz}
t port-float {socket 127.0.0.1 8.5}
t port-neg {socket 127.0.0.1 -1}
t port-huge {socket 127.0.0.1 99999}
t port-huge-async {socket -async 127.0.0.1 99999}
t port-zero-refused {socket 127.0.0.1 0}
# ── sync 拒连 ──
t sync-dead {socket 127.0.0.1 1}
# ── async 死路终态（ENOTCONN 面）──
t async-dead {set d [socket -async 127.0.0.1 1]; string match sock* $d}
t async-blocking {fconfigure $d -blocking}
t async-trans [list fconfigure $d -translation]
t async-connecting {fconfigure $d -connecting}
t async-puts-buffered {puts $d x}
t async-flush-dead {flush $d}
t async-gets-dead {gets $d}
t async-read-dead {read $d}
t async-flush2-dead {flush $d}
t async-peer-dead {fconfigure $d -peername}
t async-sockname-e0 {lindex [fconfigure $d -sockname] 0}
t async-sockname-len {llength [fconfigure $d -sockname]}
t async-close {close $d}
# ── socket 的 fconfigure 选项表 ──
t sock-bad-opt {set e [socket -async 127.0.0.1 1]; fconfigure $e -nosuch}
t sock-set-peer {fconfigure $e -peername x}
t sock-prefix-peer {fconfigure $e -peer}
t sock-prefix-block {fconfigure $e -bloc}
t sock-full-q-err {catch {fconfigure $e} m; set m}
t sock-opts-arity {fconfigure $e}
close $e
t file-bad-opt {set f [open /tmp/rtcl_p23_tmp.txt w]; set r [catch {fconfigure $f -nosuch} m]; close $f; list $r $m}
exit 0
