# jit-battery: preexisting — trace callback output shape differs tclsh vs rtcl (记录分歧); jit==rtcl enforced.
trace add variable t write {puts traced}
lappend t 1
puts $t
