foreach e {0x1.8p3 1.8x 1.p 5x Influence inf 3eq2 {1 2} 1+ 1.2.3 0x1.8 .8 . x} {
  if {[catch {expr $e} m]} { puts "ERR <$e> ==> $m" } else { puts "OK  <$e> ==> $m" }
}
