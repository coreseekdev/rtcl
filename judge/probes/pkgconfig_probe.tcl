# Probe: ::tcl::pkgconfig decoded semantics vs tclsh 8.6.17 (gen_config cluster).
# Run with both /usr/bin/tclsh and ./target/release/rtcl; outputs must match.
# Decoded rules (see misc.rs cmd_pkgconfig comments):
#   - usage messages embed the INVOKED spelling (objv[0])
#   - arity: no sub / get+2 extras -> generic "subcommand ?arg?";
#     list+extra -> "cmd list"; get alone -> "cmd get key"
#   - bad subcommand: TCL LOOKUP INDEX subcommand <x>
#   - unknown key: "key not known" + TCL LOOKUP CONFIG <x> (case-sensitive)
#   - value keys mirror the reference build (Debian tcl8.6 8.6.17)

foreach script {
    {::tcl::pkgconfig}
    {::tcl::pkgconfig foo}
    {::tcl::pkgconfig list foo}
    {::tcl::pkgconfig list a b}
    {::tcl::pkgconfig list a b c}
    {::tcl::pkgconfig get}
    {::tcl::pkgconfig get foo}
    {::tcl::pkgconfig get foo bar}
    {::tcl::pkgconfig get a b c}
    {::tcl::pkgconfig get LIST}
    {::tcl::pkgconfig get ""}
    {::tcl::pkgconfig get "a b"}
    {::tcl::pkgconfig ""}
    {::tcl::pkgconfig "a b"}
    {::tcl::pkgconfig l}
    {::tcl::pkgconfig ge}
    {::tcl::pkgconfig GET}
    {tcl::pkgconfig}
    {tcl::pkgconfig list foo}
} {
    set c [catch $script m]
    puts "script=$script c=$c msg=<$m> ec=<$::errorCode>"
}

# Success surface: key count, key list, spot values, invocation spelling.
set l [::tcl::pkgconfig list]
puts "nkeys=[llength $l]"
puts "keys=$l"
foreach k {debug threaded 64bit optimized bindir,install scriptdir,runtime} {
    puts "$k=[::tcl::pkgconfig get $k]"
}
puts "selfcmp=[string compare $l [::tcl::pkgconfig list]]"
puts "getcmp=[string compare \
    [::tcl::pkgconfig get bindir,install] \
    [::tcl::pkgconfig get bindir,install]]"

# auto_qualify (gen_init cluster) — man-page examples + colon-run collapse.
foreach {cmd ns} {::foo::bar ::blue ::global ::sub nocolons :: nocolons ::sub
                  foo::bar :: foo::bar ::sub :::foo::::bar ::blue :::foo ::bar} {
    puts "aq($cmd,$ns)=[auto_qualify $cmd $ns]"
}
