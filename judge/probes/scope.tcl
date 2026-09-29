proc probe {label script} {
    set c [catch {uplevel 1 $script} r]
    puts "$label => c=$c r=<$r>"
}
namespace eval foo {variable v 5}
proc foo::p1 {} {return $v}
probe "ns-variable, proc reads" {foo::p1}
namespace eval foo {set w 6}
proc foo::p2 {} {return $w}
probe "ns-eval-set, proc reads" {foo::p2}
probe "global, proc reads" {proc g1 {} {return $x}; set x 40; g1}
probe "proc, undefined var then local" {proc g2 {} {set y 1; return $y}; g2}
probe "proc, lappend undefined" {proc g3 {} {lappend z a; return $z}; g3; unset -nocomplain ::z; g3}
probe "proc, incr undefined" {proc g4 {} {incr q; return $q}; g4}
probe "ns proc, sees global same-tail" {namespace eval n2 {set v 9}; proc n2::p3 {} {return $v}; n2::p3}
