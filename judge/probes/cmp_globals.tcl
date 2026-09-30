# cmp_globals.tcl — standard tcl_* globals rtcl's init should define.
# gen_compile 15.3 fails only because ::tcl_library does not exist in rtcl,
# so `return $::tcl_library` errors inside catch (rc 1 instead of 2).
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

t tcl_library    {set ::tcl_library}
t tcl_version    {set ::tcl_version}
t tcl_patchLevel {set ::tcl_patchLevel}
t tcl_rcFilename {info exists ::tcl_rcFileName}
t tcl_lib_path   {namespace eval :: {set tcl_library}}
t ret_lib        {apply {{} {catch {return $::tcl_library}}}}
t ret_version    {apply {{} {catch {return $::tcl_version}}}}
t ret_missing    {apply {{} {catch {return $::nosuchglobal}}}}
t ret_body       {apply {{} {catch {return foo}}}}
t ret_plain      {apply {{} {catch return}}}
# tclsh info library / names of library-related commands
t info_library   {info library}
t pkg_v_comp     {package vcompare 8.6.17 8.6.3}
