# objsec：对象化列表的自嵌套不再退化为 O(n²) 字符串化（100k 次 list
# 自嵌套 + unset 都应在 2 秒内完成）。墙钟秒级断言有 ±1s 负载抖动
# （三次独立会话实证 flake：loop=0/1 随负载互现）——改为容差断言，
# 两段各 ≤2s 打 ok；rtcl 退化 >2s 打 SLOW，与 oracle 的 ok 形成差异
# 被门禁抓住（原缺陷：100% CPU 跑不完）。
set s0 [clock seconds]
set x {}
for {set i 0} {$i<100000} {incr i} { set x [list $x {}] }
set s1 [clock seconds]
unset x
set s2 [clock seconds]
set ok [expr {($s1-$s0) <= 2 && ($s2-$s1) <= 2}]
puts "objsec=[expr {$ok ? {ok} : {SLOW}}]"
puts DONE
