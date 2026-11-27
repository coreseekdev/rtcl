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

# classvariable is a METHOD-SCOPE command (not an oo::define subcommand);
# properties are provided by the oo::configurable metaclass.
q classvariable-shared {
    oo::class create CountP {
        variable number
        constructor {} {classvariable count; set number [incr count]}
        method report {} {classvariable count; list $number $count}
    }
    set a [CountP new]
    set b [CountP new]
    set ra [$a report]
    set rb [$b report]
    set c [CountP new]
    list $ra $rb [$c report]
}
q classvariable-in-define {oo::class create CV {}; list [catch {oo::define CV {classvariable cv 5}} m] $m}
q property-configurable {oo::configurable create PropP {property foo}; info class properties PropP}
q property-plain-class {list [catch {oo::class create PlainP {property foo}} m] $m}
q property-kinds {
    oo::configurable create KindP {property a -kind readable; property b -kind writable; property c -kind readwrite}
    list [info class properties KindP] [info class properties KindP -readable] [info class properties KindP -writable]
}
q configure {
    KindP create kp
    kp configure -c 1
    kp configure -c
}

# object identity / introspection
q creationid {set o [oo::object new]; list [expr {[info object creationid $o] > 0}] [expr {[info object creationid $o] == [info object creationid $o]}]}
q isa {oo::class create IsaP {}; IsaP create i; list [info object isa object i] [info object isa class i] [info object isa class IsaP] [info object isa typeof i IsaP]}
q vars {oo::class create VarsP {variable a}; list [info class variables VarsP]}
