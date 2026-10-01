# agent-e-loops: decoded tclsh 8.6.17 semantics of `return -level 0` in
# loop bodies and of other completion codes reaching lmap/foreach bodies.
# Oracle output (label|catchCode|result):
#   return -level 0 v    -> body completes NORMALLY with result v: lmap
#     collects it and keeps iterating; foreach/for treat it as OK;
#     `while {1} { return -level 0 x }` loops forever.
#   return -level 0      -> same, with an empty result (collected as {}).
#   return -code return / -code break / -code continue / -code error
#     (default -level 1) -> catch sees code 2 and the value; conversion
#     to the requested code happens at the enclosing proc boundary.
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

t lm-collect-all {lmap i {a b c} { return -level 0 $i }}
t lm-keeps-going {lmap i {a b c} { if {$i eq "b"} { return -level 0 $i }; set _ }}
t lm-empty {lmap i {a b c} { return -level 0 }}
t lm-code-return {lmap i {a b c} { return -code return $i }}
t lm-code-break {lmap i {a b c} { return -code break $i }}
t lm-code-continue {lmap i {a b c} { return -code continue $i }}
t lm-code-error {lmap i {a b c} { return -code error $i }}
t lm-return-in-proc {proc p {} { lmap i {a b c} { return $i } }; p}
t lm-code-return-level0 {proc p {} { lmap i {a b c} { return -code return -level 0 $i } }; p}
t fe-level0 {proc p {} { foreach i {a b c} { return -level 0 $i }; set done 1 }; p}
t fe-code-return {proc p {} { foreach i {a b c} { return -code return $i }; set done 1 }; p}
t fe-level0-keeps-going {proc p {} { foreach i {a b c} { if {$i=="b"} {return -level 0 $i} } ; list after $i }; p}
t for-level0 {proc p {} { for {set i 0} {$i<3} {incr i} { return -level 0 $i } ; list after $i }; p}
# t while-level0 {proc p {} { while {1} { return -level 0 x } }; p}  # loops forever in tclsh
