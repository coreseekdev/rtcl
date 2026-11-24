# smallface_p24_open — the two smallface_p24 shapes still open, kept apart
# so smallface_p24 stays an exact gate.  Tracked in
# specs/CODE-AUDIT-20261123.md; move a shape back once it matches.
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|$::errorCode" }
set ::errorCode NONE

# C17: `lt`/`le`/`gt`/`ge` are not operators in tclsh 8.6 (`lt` there is a
# bareword); rtcl compares strings with them.
catch {expr {1 lt}} m9 o9
puts "x5|[dict get $o9 -errorcode]"

# C16: the frame of an arithmetic error.  tclsh words it `invoked from
# within` for an expression made of literals alone (folded when compiled)
# and `while executing` once a runtime value takes part; rtcl's bytecode
# engine always says the one and its tree walker the other.
catch {expr {1/0}} ma oa
puts "a1|[dict get $oa -errorinfo]|[dict get $oa -errorstack]"
