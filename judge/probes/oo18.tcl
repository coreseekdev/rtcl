# 1. memo hit path: repeated calls
oo::class create C { method m x { expr {$x * 2} } }
C create c
set r1 [list [c m 1] [c m 2] [c m 3]]
# 2. redefinition mid-flight: new body must run
oo::define C method m x { expr {$x * 3} }
set r2 [c m 4]
# 3. export toggling: unexport then call from outside -> unknown
oo::define C method helper {} { return hidden }
set r3 [catch {c helper} e3]
oo::define C unexport m
set r4 [catch {c m 1} e4]; set m4 $e4
# my still reaches unexported
set r5 ""
oo::define C method callmy {} { my m 7 }
oo::define C export m
set r5 [c callmy]
# 4. mixin added after memo filled
oo::class create Mx { method m x { expr {$x * 100} } }
C create c2
set r6a [c2 m 1]
oo::define C mixin Mx
set r6b [c2 m 1]
oo::define C mixin
set r6c [c2 m 1]
# 5. superclass chain change
oo::class create Base { method inherited {} { return base-v1 } }
oo::class create Mid { superclass Base }
Mid create mobj
set r7a [mobj inherited]
oo::define Base method inherited {} { return base-v2 }
set r7b [mobj inherited]
# 6. next after redefinition follows the invocation-time chain
oo::class create P1 { method n {} { return p1-orig } }
oo::class create P2 { superclass P1; method n {} { return [next]-p2 } }
P2 create p2obj
set r8a [p2obj n]
oo::define P1 method n {} { return p1-new }
set r8b [p2obj n]
# 7. deletemethod / renamemethod
oo::class create D { method a {} { return A }; method b {} { return B } }
D create d
set r9a [d a]
oo::define D deletemethod a
set r9b [catch {d a} e9b]; set m9b $e9b
oo::define D renamemethod b bb
set r9c [catch {d b} e9c]; set r9d [d bb]
# 8. destroy cascades
oo::class create Killer { superclass Base }
Killer create kv
set r10a [catch {kv inherited} e10a]
Base destroy
set r10b [catch {kv inherited} e10b]
# 9. unexport-of-inherited (hidden)
oo::class create HBase { method pub {} { return pub } }
oo::class create HSub { superclass HBase }
HSub create hs
set r11a [hs pub]
oo::define HSub unexport pub
set r11b [catch {hs pub} e11b]; set m11b $e11b
puts [list $r1 $r2 $r3 $r4:$m4 $r5 $r6a:$r6b:$r6c $r7a:$r7b $r8a:$r8b $r9a:$r9b:$m9b:$r9c:$r9d $r10a:$r10b $r11a:$r11b:$m11b]
