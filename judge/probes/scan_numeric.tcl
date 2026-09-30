foreach {fmt s} {
  %i 0x  %i 0b101  %i 0o17  %i 017  %i 0X1A  %x 0xff  %x ff  %b 0b101  %o 0o17  %o 017
  %d 1_000  %i 1_000  %f 3.  %f .5  %f 1e3  %f e3  %f 1e  %f +.5  %f 1.2.3
  %d -5 %u -1  %i -0x10  %x -ff  %d 99999999999999999999  %f 12345678901234567890
} {
    if {[catch {scan $s $fmt} r]} { puts "$fmt <$s> ERR $r" } else { puts "$fmt <$s> -> $r" }
}
