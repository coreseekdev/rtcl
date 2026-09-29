# catch / error / errorCode
catch {error "boom"} msg
puts "caught: $msg"
catch {error "boom"} msg opts
puts [dict get $opts -code]
puts [catch {error "x"}]
puts [catch {expr {1+1}} result]
puts $result
catch {expr {1/0}} divmsg
puts $divmsg
if {[catch {undefined_command_xyz} e]} { puts "undef: $e" }
# catch with return/break propagation
proc trybreak {} {
    set out {}
    catch {
        for {set i 0} {$i < 5} {incr i} {
            if {$i == 2} break
            lappend out $i
        }
    }
    return $out
}
puts [trybreak]
