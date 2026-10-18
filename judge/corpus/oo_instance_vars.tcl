# Instance-variable isolation across objects of one class: the method
# memo's prelink resolution must be keyed by the object's own namespace,
# not merely the class's variable list (one MethodDef serves every
# instance).
oo::class create C {
    variable acc
    method m {} { incr acc; return $acc }
    method g {} { return $acc }
}
C create c1
C create c2
puts "[c1 m] [c1 m] [c2 m] [c1 g] [c2 g]"
puts [catch {oo::class create D {variable v; method g {} {return $v}}; D create d; d g} msg]
puts $msg
# Writes stay live through the link (external view of the same variable).
puts [c1 g]
