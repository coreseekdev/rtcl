# set / subst / quoting semantics
set a hello
puts $a
puts "$a world"
puts {$a world}
set b "a"
puts $$b
puts [set a]
set x {1 2 3}
puts $x
puts "len=[llength $x]"
# backslash escapes
puts "tab\tend"
puts "newline\nend"
puts "\$literal"
puts [expr {2+3}]
# command substitution nesting
puts [string toupper [string range "hello world" 0 4]]
