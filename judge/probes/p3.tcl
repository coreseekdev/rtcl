set got {}
proc foo {} {}
trace add command foo rename {lappend ::got}
trace add command foo delete {lappend ::got}
rename foo bar
rename bar {}
puts "'$got'"
puts [llength $got]
