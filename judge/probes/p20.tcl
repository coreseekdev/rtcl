set r {}
append r [switch -nocase -glob HE* hello {set r one} hell* {set r two} default {set r three}]
puts $r
puts [switch x a {subst 1} default {subst 2} c {subst 3} default {subst 4}]
puts [switch -nocase b a {subst 1} b {subst 2} c {subst 3} default {subst 4}]
