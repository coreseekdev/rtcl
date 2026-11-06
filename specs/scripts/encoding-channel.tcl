proc hex {s} { binary encode hex $s }
proc dump {p} { set f [open $p r]; fconfigure $f -translation binary; set d [read $f]; close $f; hex $d }
# 写：多行/多 put
foreach enc {iso2022-jp iso2022-kr iso2022} {
    set f [open /tmp/ec_w.txt w]
    fconfigure $f -encoding $enc
    puts $f "\u3042"
    puts -nonewline $f "\u3044A"
    close $f
    puts "w-$enc: [dump /tmp/ec_w.txt]"
}
# 读：gets/read
foreach enc {iso2022-jp iso2022-kr} {
    set f [open /tmp/ec_r.txt w]
    fconfigure $f -translation binary
    if {$enc eq "iso2022-jp"} {
        puts -nonewline $f [binary format H* 1b2442242224241b2842]
    } else {
        puts -nonewline $f [binary format H* 1b2429430e71692a220f]
    }
    close $f
    set f [open /tmp/ec_r.txt r]
    fconfigure $f -encoding $enc
    set a [gets $f]
    set b [read $f]
    close $f
    puts "r-$enc: [string length $a]|[binary encode hex [encoding convertto utf-8 $a]]|[binary encode hex [encoding convertto utf-8 $b]]"
}
file delete /tmp/ec_w.txt /tmp/ec_r.txt
