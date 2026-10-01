set ::log {}
proc h {method args} {
  lappend ::log "$method $args"
  switch -- $method {
    initialize { return {initialize finalize watch read write} }
    read { return "DATA" }
    default { return "" }
  }
}
set r [catch {chan create {r w} h} ch]
puts "create|$r|$ch"
set ::log {}
set r [catch {read $ch 2} m]
puts "read2|$r|$m"
puts "LOG2=$::log"
set ::log {}
set r [catch {eof $ch} m]
puts "eof|$r|$m"
set ::log {}
set r [catch {close $ch} m]
puts "close|$r|$m"
puts "LOG3=$::log"
set r [catch {chan names} m]
puts "names|$r|$m"
