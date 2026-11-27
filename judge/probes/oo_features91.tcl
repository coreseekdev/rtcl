# oo_features91 — TclOO 9.1.0 surface: abstract / singleton / private /
# classmethod / class variables / properties / creationid / isa.
# Pins what the 9.1.0 binary actually provides; docs ship some 9.2 features
# (classvariable, core property) that are absent here.

proc q {label script} {
    set c [catch {uplevel 1 $script} r]
    puts "$label|$c|$r"
}

# abstract: create/new removed, subclasses still instantiate
q abstract-create {oo::abstract create AbsP {method m {} {return m}}; list [catch {AbsP create x} m] $m}
q abstract-new {list [catch {AbsP new} m] $m}
q abstract-sub {oo::class create AbsSub {superclass AbsP}; AbsSub create s; s m}

# singleton: new is the single accessor; create is not offered
q singleton-same {
    oo::singleton create SingP {method m {} {return m}}
    set a [SingP new]
    set b [SingP new]
    expr {$a eq $b}
}
q singleton-create {list [catch {SingP create z} m] $m}

# private method: callable via my, invisible outside
q private-method {
    oo::class create PrivP {}
    oo::define PrivP {private method secret {} {return s}; method pub {} {return [my secret]}}
    PrivP create p
    list [p pub] [catch {p secret} m] $m
}

# classmethod: callable on the class object
q classmethod {oo::class create ClsP {classmethod cm {} {return ok}}; list [ClsP cm]}

# class variables: public/private split in introspection
q classvars {
    oo::class create VarP {variable x; private variable y}
    list [info class variables VarP] [info class variables VarP -private]
}

# 9.1.0 gaps: docs exist but the definition commands do not
q classvariable-absent {
    oo::class create CV {}
    list [catch {oo::define CV {classvariable cv 5}} m] $m
}
q property-absent {list [catch {oo::class create PropP {property foo}} m] $m}

# object identity / introspection
q creationid {set o [oo::object new]; list [expr {[info object creationid $o] > 0}] [expr {[info object creationid $o] == [info object creationid $o]}]}
q isa {oo::class create IsaP {}; IsaP create i; list [info object isa object i] [info object isa class i] [info object isa class IsaP] [info object isa typeof i IsaP]}
q vars {oo::class create VarsP {variable a}; list [info class variables VarsP]}
