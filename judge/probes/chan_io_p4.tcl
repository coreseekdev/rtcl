proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t create-badmode-c {proc foo {} {}; set r [catch {chan create {c} foo} m]; rename foo {}; list $r $m}
t create-nocmd {catch {chan create {r w} foo} m; set m}
t create-wrongargs {proc foo {} {}; set r [catch {chan create {r w} foo} m]; rename foo {}; list $r $m}
t create-wrongargs-ns {proc foo {} {}; set r [catch {chan create {r w} ::foo} m]; rename foo {}; list $r $m}
t create-handler-args {proc foo {cmd args} {lappend ::log $cmd $args}; set r [catch {chan create {r w} foo} m]; rename foo {}; list $r $m}
t create-mode-rw-list {catch {chan create {r w} {}} m; set m}
t create-mode-scalar {catch {chan create rw {}} m; set m}
t create-mode-empty {catch {chan create {} {}} m; set m}
t create-mode-weird {catch {chan create {read WRITE} {}} m; set m}
