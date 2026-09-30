namespace eval ns {
    proc foo {} {
        variable x 1
        set ::dbg [list exist-before=[info exist x]]
        bar
        set ::dbg2 [list exist-after=[info exist x] val=[catch {set x} v] v=$v]
        info exist x
    }
    proc bar {} { namespace delete [namespace current] }
    namespace export *
    namespace ensemble create
}
puts "R:[ns foo]"
puts "D1:$::dbg"
puts "D2:$::dbg2"
