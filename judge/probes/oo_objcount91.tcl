# oo_objcount91 — TclOO object identity/counter semantics (tclsh 9.1.0).
#
# Object names are ::oo::Obj<N> with a process-global counter.  The core
# pre-creates 20 OO objects, so a fresh interpreter's first user object is
# ::oo::Obj20 (creationid 20); named objects and classes share the counter.
# Semantics pinned:
#   - first user object is ::oo::Obj20 / creationid 20
#   - anonymous names match ::oo::Obj<digits>
#   - info object creationid is the suffix and is stable for an object
#   - creation ids are strictly increasing, including after destroy
# Run this file in a fresh tclsh; no OO objects may be created before `base`.

proc t {label script} {
    set c [catch {uplevel 1 $script} r]
    puts "$label|$c|$r"
}

t base {set o [oo::object new]; list $o [info object creationid $o]}
t name-shape {string match {::oo::Obj*} [oo::object new]}
t id-stable {set o [oo::object new]; list [info object creationid $o] [info object creationid $o]}
t id-is-suffix {
    set o [oo::object new]
    list [info object creationid $o] [regsub {^::oo::Obj} $o {}]
}
t id-increases {
    set a [oo::object new]
    set ia [info object creationid $a]
    set b [oo::object new]
    set ib [info object creationid $b]
    list [expr {$ib == $ia + 1}] [expr {$ib > $ia}]
}
t id-after-destroy {
    set a [oo::object new]
    set ia [info object creationid $a]
    $a destroy
    set b [oo::object new]
    list [expr {[info object creationid $b] > $ia}]
}
t class-id {
    oo::class create ObjCountClass
    set c ObjCountClass
    set i [info object creationid $c]
    list [expr {$i > 0}] [expr {[info object creationid $c] == $i}]
}
t named-object {oo::object create ObjCountNamed; info object creationid ObjCountNamed}
t named-class {oo::class create ObjCountNamedClass; expr {[info object creationid ObjCountNamedClass] > 0}}
