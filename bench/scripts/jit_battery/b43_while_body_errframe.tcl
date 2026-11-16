# while 体错误帧族（fuzz f000022 最小化）：体区 unwind 的
# ("while" body line N) + 站点 harness + invoked-from-within 链。
set m 0
while {$m < 5} {
  incr m
  lappend b $a
}
