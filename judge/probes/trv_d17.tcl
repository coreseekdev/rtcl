proc callback {old - -} { puts "CB-BARE $old" }
namespace eval ::foo {proc bar {} {}}
trace add command ::foo::bar delete callback
namespace delete ::foo
puts "done1"

proc cb2 {old - -} { puts "CB-INSCOPE $old" }
namespace eval ::foo2 {proc bar {} {}}
trace add command ::foo2::bar delete [namespace code cb2]
puts "script2=[namespace code cb2]"
namespace delete ::foo2
puts "done2"

proc cb3 {old - -} { puts "CB-LIST $old" }
namespace eval ::foo3 {proc bar {} {}}
trace add command ::foo3::bar delete {cb3}
namespace delete ::foo3
puts "done3"

proc cb4 {args} { puts "CB-VARARG $args" }
namespace eval ::foo4 {proc bar {} {}}
trace add command ::foo4::bar delete [list cb4]
namespace delete ::foo4
puts "done4"
