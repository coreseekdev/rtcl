# C38（9.1 对齐批 B1，2026-11-26）：别名目标 wrong-args 消息的命令名。
# oracle（tclsh 9.1.0）实测——tclsh 8.6 自身不一致（父目标写别名名、
# 兄弟目标写目标名），9.1 统一：
# - wrong-args 消息命令名 = **别名调用词**（原 rtcl 一律目标名）；
# - 带前缀实参的别名：消息到名字为止（should be "x6"）；
# - 无前缀：带目标参数表（should be "n6 a b"）；
# - interp alias 的 usage 行也换 9.1 用语：childPath childCmd
#   ?parentPath parentCmd?（原 rtcl slave/master）。
# 错误码 TCL WRONGARGS、跨解释器转发语义（前缀+调用实参拼接）不变。
interp create child
interp eval child {proc t6 {a b} {return x}}
interp create sib
interp eval sib {proc t6 {a b} {return x}}
interp alias child pc child t6
catch {interp eval child {pc onlyone}} m e
puts "A parent-nopref: m=$m ec=[dict get $e -errorcode]"
interp alias child sc sib t6
catch {interp eval child {sc onlyone}} m e
puts "B sibling-nopref: m=$m ec=[dict get $e -errorcode]"
interp alias child c6 child t6 aa bb
puts "C same-pref-3: [catch {interp eval child {c6 1 2}} m] m=$m"
interp alias child x6 sib t6 aa bb
puts "D cross-pref-0: [catch {interp eval child {x6}} m] m=$m"
puts "E cross-pref-1: [catch {interp eval child {x6 1}} m] m=$m"
puts "F cross-pref-2: [catch {interp eval child {x6 1 2}} m] m=$m"
interp alias child n6 sib t6
puts "G cross-nopref-1: [catch {interp eval child {n6 1}} m] m=$m"
puts "H cross-nopref-3: [catch {interp eval child {n6 1 2 3}} m] m=$m"
interp eval child {proc t7 {args} {return "t7:[llength $args]:$args"}}
interp alias child b7 child t7 pp
puts "I same-pref-fwd: [catch {interp eval child {b7 x}} m] m=$m"
proc targs2 {a b} { return "ok:$a$b" }
interp alias {} m1 {} targs2
catch {m1 one} m
puts "J main-nopref: m=$m"
interp alias {} m2 {} targs2 pre
catch {m2 one} m
puts "K main-pref: m=$m"
catch {interp alias child q set} m
puts "L usage: m=$m"
interp alias child ok2 child t6 aa bb
catch {interp eval child {ok2 aa bb}} m
puts "M ok-args: m=$m"
