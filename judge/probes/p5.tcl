foreach e {{-0x1234} {"0005"} {"abc"} {"0x10"} {{1 2}} {"3.5"} {"0005"zxy} {"0x1A"+"1"} {{- 5}} {0x10+"0x10"} {{}} {" 1"}} {
    if {[catch {expr $e} msg]} { puts "$e => ERR: $msg" } else { puts "$e => '$msg'" }
}
