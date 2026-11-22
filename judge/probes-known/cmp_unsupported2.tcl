proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t dn_colon  {tcl::unsupported::disassemble proc ::nosuchproc}
t dp_exists {proc realp {x} {return $x}; tcl::unsupported::disassemble proc realp}
t dd_0arg   {tcl::unsupported::disassemble destructor}
t dd_extra  {tcl::unsupported::disassemble destructor a b}
t gd_0arg   {tcl::unsupported::getbytecode destructor}
t dsc_0arg  {tcl::unsupported::disassemble script}
t gsc_0arg  {tcl::unsupported::getbytecode script}
t dl_extra  {tcl::unsupported::disassemble lambda {{} {set x 1}} extra}
t gm_oo_0arg {tcl::unsupported::getbytecode method oo::object}
t gc_oo     {tcl::unsupported::getbytecode constructor oo::object}
t gd_oo     {tcl::unsupported::getbytecode destructor oo::object}
t dc_obj    {tcl::unsupported::disassemble constructor nosuchclass extra}
t gob_oo1   {tcl::unsupported::disassemble objmethod oo::object}
t proc_in_ns {namespace eval qq {proc pp {} {return 1}}; tcl::unsupported::disassemble proc qq::pp}
