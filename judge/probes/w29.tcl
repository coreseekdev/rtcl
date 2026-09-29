set __judge_c [catch {
    tcl_startOfNextWord "ab cd" -0
} __judge_r]
puts "$__judge_c $__judge_r"
