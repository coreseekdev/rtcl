foreach f {{%} {%1$} {%1$d %2} {%5} {%ld} {%*} {%1$d %1$d} {%d } {%s %s} {x%o} {[} {[]} {[^]} {%[ab} {%hu d} {%2c} {%c2}} {
    set r [catch {scan abcdefgh $f v1 v2 v3} msg]
    set ec [catch {scan abcdefgh $f v1 v2 v3; set ec2 x} junk]
    catch {scan abcdefgh $f v1 v2 v3} m2
    puts "== $f -> code=$r msg=$msg"
}
# nconversions/results
puts [scan abc {%2c%n} a b]|$a|$b
puts [scan abcdef {%*2c%n} b]; puts $b
puts [scan abc {%s%n%s} a b c]; puts "a=$a b=$b c=[info exists c]"
puts [scan abc {%1$s%1$s} a]
catch {scan abc {%s%s} a} m; puts $m
catch {scan abc {%s%d} a b} m; puts "$m | [scan abc {%s%d} a b]"
puts [scan 12ab {%x%n} h n]; puts "$h $n"
puts [scan 12 {%i%d} a b]; puts "$a $b"
puts [scan abc {%[a-c]} x]; puts $x
puts [scan abc {[^b]} x]; puts $x
puts [scan a-b {[%-]} x]; puts $x
puts [scan abc {%c%c%c} x y z]; puts "$x $y $z"
