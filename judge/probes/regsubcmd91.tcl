# regsub -command（TIP 463，9.1 新选项）——回调语义钉形：前缀+匹配串
# +各捕获组串（未参与=空元素）按值求值；& 与 \n 失去模板含义；错误
# 原样上抛；空前缀 TCL OPERATION REGSUB CMDEMPTY；-all 逐匹配回调。
proc ec {e} {expr {[dict exists $e -errorcode] ? [dict get $e -errorcode] : {}}}
proc try {script} {set c [catch {uplevel 1 $script} m e]; return "$c|$m|[ec $e]"}
puts [regsub -command {.x.} {abcxdef} {string length}]
puts [regsub -command {.x.} {abcxdefxghi} {string length}]
puts [regsub -all -command {.x.} {abcxdefxghi} {string length}]
puts [regsub -nocase -command {X.} {abcxdef} {string length}]
puts [regsub -command {(b)c} {abcxdef} {list}]
puts [regsub -command {(.)(x)} {abcxdef} {list}]
puts [regsub -all -command {(b)|(z)} {abcxdef} {list}]
set v {}
puts "[regsub -command {.x.} {abcxdefxghi} {string length} v] [set v]"
puts [try {regsub -command {b} {abcxdef} {} v}]
puts [try {regsub -command {.x.} {abcxdef} {error boom}}]
# 模板含义关闭：& 与 \1 原样传给回调
puts [regsub -command {b} {abcxdef} {list & \\1 &}]
puts DONE
