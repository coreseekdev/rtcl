# 空转（T11b 普查 2026-11-26）：两边死在行7：`string match {a[} "a["` 残缺字符集报错，行8–13 未跑
puts [string match {[a-z]*} "aa"]
puts [string match {[a-z]*} "Ab"]
puts [string match {[abc]x} "bx"]
puts [string match {[a-c]x} "bx"]
puts [string match {[^a]x} "bx"]
puts [string match {[!a]x} "bx"]
puts [string match {a[} "a["]
puts [string match {a[b} "a[b"]
puts [string match {[]x]y} "]y"
puts [string match {[a-]z} "-z"]
puts [string match {[\]]z} "]z"
puts [string match {[a\]z} "az"
puts [info patchlevel]
