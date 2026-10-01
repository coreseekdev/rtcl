if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t q-len {string length [fconfigure stdin -eofchar]}
t q-lindex {lindex [fconfigure stdin -eofchar] 0}
t q-lindex1 {lindex [fconfigure stdin -eofchar] 1}
t push-nochan {proc h {m a} {return {initialize finalize read write}}; catch {chan push gorp h} e; set e}
t push-log {set ::l {}; proc h {m args} {lappend ::l $m $args; return {initialize finalize read write watch}}; set ch [open /tmp/rtcl_probe.txt w]; set r [catch {chan push $ch h} e]; close $ch; list $r $e $::l}
t push-badprefix {set ch [open /tmp/rtcl_probe.txt w]; set r [catch {chan push $ch "x \{"} e]; close $ch; list $r $e $::errorCode}
t create-ret-empty {proc h {m a} {return {}}; set r [catch {chan create {r} h} e]; list $r $e}
t create-ret-read {proc h {m a} {return {read}}; set r [catch {chan create {r} h} e]; list $r $e}
t create-ret-drain {proc h {m a} {return {drain}}; set r [catch {chan create {r} h} e]; list $r $e}
t create-ret-iniz {proc h {m a} {return {iniz}}; set r [catch {chan create {r} h} e]; list $r $e}
t prefix-clo {catch {chan clo stdin} e; set e}
t prefix-co {catch {chan co stdout} e; set e}
t prefix-c {catch {chan c stdout} e; set e}
t prefix-n {catch {chan n} e; set e}
t prefix-p {catch {chan p} e; set e}
t names-pattern {chan names *out*}
