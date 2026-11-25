# 空转（T11b 普查 2026-11-26）：两边死在末行19：`puts "R=[$__judge_r]"` 多一对括号把结果当命令调用（invalid command name "0 wolf {x Y read}"），R= 从未打印
# case: trace-1.7 cmd=unset
set __judge_c [catch {
    unset -nocomplain x
    set info {}
    trace add variable x read q
    proc q {name1 name2 op} {
	global info
	set info [list $name1 $name2 $op]
	global $name1
	set ${name1}($name2) wolf
    }
    proc p {} {
	global x
	set x(X) willi
	return $x(Y)
    }
    list [catch {p} msg] $msg $info
} __judge_r]
puts "R=[$__judge_r]"
