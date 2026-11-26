# switch -integer (tclsh 9.1 new option) — behavior pins.
# Source: tclCmdIL.c Tcl_SwitchObjCmd SwitchIntegerMatch + option table.
catch {switch -foo a b c} m; puts "1:$m :: $errorCode"
catch {switch -glob -integer Foo Foo {set result OK}} m; puts "2:$m :: $errorCode"
catch {switch -integer -glob Foo Foo {set result OK}} m; puts "3:$m :: $errorCode"
catch {switch -integer -nocase Foo Foo {set result OK}} m; puts "4:$m :: $errorCode"
catch {switch -nocase -integer Foo Foo {set result OK}} m; puts "5:$m :: $errorCode"
puts [switch -integer 05 {5 {return B}}]
catch {switch -integer x {5 {return C}}} m; puts "6:$m :: $errorCode"
puts [switch -integer -1 {-1 {return D}}]
catch {switch -integer 1.5 {1 {return E}}} m; puts "7:$m :: $errorCode"
catch {switch -integer 99999999999999999999999 {1 {return F}}} m; puts "8:$m :: $errorCode"
puts [switch -integer 0x1 01 {string cat ok}]
puts [switch -integer 0x1 0 {error early} 01 {string cat ok}]
puts [switch -integer 0x1 0 {error early} 01 {string cat ok} default {error late}]
puts [switch -integer 007 {7 {return G}}]
puts [switch -integer 0x10 {16 {return H}}]
puts [switch -integer +5 {5 {return I}}]
puts [switch -int 05 {5 {return J}}]
catch {switch -integer zz {x {set b 1} default {set b 2}}} m; puts "11:$m :: $errorCode"
catch {switch -integer 5.0 {5 {set d 1} default {set d 2}}} m; puts "12:$m :: $errorCode"
puts [switch -integer 9 5 {return A} default {return D}]
puts [switch -integer 9 5 {return A} 6 {return B}]
puts [switch -integer 05 {5 {return OK}}]
puts [apply {{} { switch -integer 05 {5 {return OK}} }}]
catch {switch -integer 9 {x {set a 1} default {set a 2}}} m; puts "15:$m :: $errorCode"
