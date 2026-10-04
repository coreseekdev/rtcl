# OO method-body memo smoke: exercises the memoised path (no `variable`
# decls), the rebuild path (with), redefinition, next/my, exports,
# forwards, ctor/dtor, copy/<cloned>, arity errors, info level 0.
set out {}

# 1. plain method (memo path)
oo::class create Animal {
    method speak {n} {
        return "[self] says $n"
    }
    method twice {} { return [my speak x][my speak y] }
}
Animal create a1
Animal create a2
append out [a1 speak woof] | [a2 speak moo] | [a1 twice] \n

# 2. variable-linked method (rebuild path) + per-object variables
oo::class create Counter {
    variable count
    constructor {} { set count 0 }
    method bump {by} { incr count $by; return $count }
    method peek {} { return $count }
}
Counter create c1
Counter create c2
c1 bump 5; c1 bump 7; c2 bump 1
append out [c1 peek] [c2 peek] \n

# 3. object method + objdefine variable (rebuild, per-object owner)
oo::class create Blank {}
Blank create b1
oo::objdefine b1 {
    variable tag
    method set-tag {v} { set tag $v }
    method describe {} { return "tag=$tag" }
}
b1 set-tag hi
append out [b1 describe] \n

# 4. inheritance + next
oo::class create Base { method m {} { return base } }
oo::class create Mid { superclass Base; method m {} { return "mid>[next]" } }
oo::class create Leaf { superclass Mid; method m {} { return "leaf>[next]" } }
Leaf create L
append out [L m] \n

# 5. redefinition mid-flight (memo must retire with the old MethodDef)
oo::class create R { method v {} { return old } }
R create r1
append out [r1 v] ,
oo::define R method v {} { return new }
append out [r1 v] \n

# redefine between calls of two live objects
oo::class create R2 { method w {} { return 1 } }
R2 create r2a; R2 create r2b
append out [r2a w][r2b w] ,
oo::define R2 method w {} { return 2 }
append out [r2a w][r2b w] \n

# 6. export/unexport toggling after memo fill
oo::class create E { method pub {} { return visible }; method hidden2 {} { return h } }
E create e1
append out [catch {e1 hidden2} msg1] [e1 pub] ,
oo::define E unexport pub
append out [catch {e1 pub} msg2] ,
oo::define E export pub
append out [e1 pub] \n

# 7. arity error text + info level 0 (synthetic param feeds both)
oo::class create A { method m {x y} { return "$x-$y" } }
A create a3
append out [catch {a3 m 1} m3] $m3 \n
oo::class create L0 { method frame {} { return [info level 0] } }
L0 create l0
append out [l0 frame] \n

# 8. forward
oo::class create Fwd { forward len string length }
Fwd create f1
append out [f1 len abcdef] \n

# 9. constructor args + destructor log
oo::class create WithC {
    variable name
    constructor {n} { set name $n }
    method getn {} { return $name }
    destructor { global out; append out "dtor($name) " }
}
WithC create w1 bob
append out [w1 getn] ,
w1 destroy
append out \n

# 10. copy (fresh object shares the memoised compiled body)
oo::class create Cp { method id {} { return [self] } }
Cp create cp1
oo::copy cp1 cp2
append out [string match ::cp2 [cp2 id]] \n

# 11. unknown method message (exported listing)
oo::class create U { method alpha {} {return 1}; method beta {} {return 2} }
U create u1
append out [catch {u1 zzz} um] $um \n

# 12. recursion through memoised method (shared ProcDef, nested frames)
oo::class create Rec { method down {n} { if {$n <= 0} { return 0 } { return [my down [expr {$n-1}]] } } }
Rec create rc
append out [rc down 50] \n

# 13. method body error framing
oo::class create Err { method boom {} { error inner } }
Err create er
append out [catch {er boom} em] [regexp {inner} $em] \n

# 14. export status of class methods via oo::define with mixed case
oo::class create MC { method Mixed {} { return M } }
MC create mc
append out [catch {mc Mixed} mm] ,
oo::define MC export Mixed
append out [mc Mixed] \n

# 15. tailcall inside a memoised method
oo::class create TC { method go {n acc} { if {$n <= 0} { return $acc } { tailcall my go [expr {$n-1}] [expr {$acc+$n}] } } }
TC create tc
append out [tc go 100 0] \n

puts -nonewline $out
