foreach {fmt s} {
  %d 3000000000  %ld 3000000000  %lld 30000000000  %ld 30000000000  %Ld 5  %hd 5  %ld abc
  %d 2147483648  %i 2147483648  %x ffffffffffffffff
  %c é  %n x  %s héllo  %[é] é
} {
    if {[catch {scan $s $fmt} r]} { puts "$fmt <$s> ERR $r" } else { puts "$fmt <$s> -> $r" }
}
puts "w:[scan abcdef {%3d}]"
puts "w2:[scan 123 {%2d%d} a b]; $a $b"
puts "ws:[scan {  a  b} {%s %s} p q]; $p $q"
