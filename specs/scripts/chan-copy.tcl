# chan copy 差分脚本：tc1-tc3 合并（translation/-size/编码矩阵/字节串对象）

# ── tc1 ──
proc hex {s} { binary encode hex $s }
proc copycase {src_mode dst_mode} {
    set f [open /tmp/tc_in.txt w]
    fconfigure $f -translation binary
    puts -nonewline $f "a\r\nb\rc\nd"
    close $f
    set s [open /tmp/tc_in.txt r]
    fconfigure $s -translation $src_mode -encoding binary
    set d [open /tmp/tc_out.txt w]
    fconfigure $d -translation $dst_mode
    set n [chan copy $s $d]
    close $s
    close $d
    set f [open /tmp/tc_out.txt r]
    fconfigure $f -translation binary
    set out [read $f]
    close $f
    puts "copy:$src_mode>$dst_mode n=$n out=[hex $out]"
}
foreach sm {auto lf cr crlf} {
    foreach dm {lf cr crlf auto} {
        copycase $sm $dm
    }
}
# -size 语义（字节 or 字符）
set f [open /tmp/tc_sz.txt w]
fconfigure $f -translation binary
puts -nonewline $f "ééé"
close $f
set s [open /tmp/tc_sz.txt r]
set d [open /tmp/tc_sz2.txt w]
fconfigure $d -translation binary -encoding binary
set n [chan copy $s $d -size 4]
close $s
close $d
set f [open /tmp/tc_sz2.txt r]
fconfigure $f -translation binary
set out [read $f]
close $f
puts "size: n=$n out=[hex $out]"
file delete /tmp/tc_in.txt /tmp/tc_out.txt /tmp/tc_sz.txt /tmp/tc_sz2.txt

# ── tc2 ──
proc hex {s} { binary encode hex $s }
proc raw {path data} {
    set f [open $path w]
    fconfigure $f -translation binary
    puts -nonewline $f $data
    close $f
}
proc dump {path} {
    set f [open $path r]
    fconfigure $f -translation binary
    set d [read $f]
    close $f
    return [hex $d]
}
# 源 UTF-8 字节，dest 各 encoding
raw /tmp/tc_e.txt "\xc3\xa9\xc3\xa9"
foreach enc {utf-8 iso8859-1 binary shiftjis} {
    set s [open /tmp/tc_e.txt r]
    set d [open /tmp/tc_eo.txt w]
    fconfigure $d -translation binary -encoding $enc
    set n [chan copy $s $d]
    close $s; close $d
    puts "dst-$enc: n=$n [dump /tmp/tc_eo.txt]"
}
# 源 iso8859-1 字节 e9，dest utf-8
raw /tmp/tc_e2.txt "\xe9\xe9"
foreach enc {utf-8 binary iso8859-1} {
    set s [open /tmp/tc_e2.txt r]
    fconfigure $s -translation binary -encoding iso8859-1
    set d [open /tmp/tc_eo2.txt w]
    fconfigure $d -translation binary -encoding $enc
    set n [chan copy $s $d]
    close $s; close $d
    puts "src8859-dst-$enc: n=$n [dump /tmp/tc_eo2.txt]"
}
# -size 语义：10 个字节的数据
raw /tmp/tc_s.txt "0123456789"
foreach size {0 1 4 10 20 -1} {
    set s [open /tmp/tc_s.txt r]
    set d [open /tmp/tc_so.txt w]; fconfigure $d -translation binary
    set n [chan copy $s $d -size $size]
    close $s; close $d
    puts "size-$size: n=$n [dump /tmp/tc_so.txt]"
}
# 多字节字符 + size（utf-8 源 6 字节/3 字符）
raw /tmp/tc_m.txt "\xc3\xa9\xc3\xa9\xc3\xa9"
foreach size {1 2 3 4 6} {
    set s [open /tmp/tc_m.txt r]
    set d [open /tmp/tc_mo.txt w]; fconfigure $d -translation binary
    set n [chan copy $s $d -size $size]
    close $s; close $d
    puts "msize-$size: n=$n [dump /tmp/tc_mo.txt]"
}
file delete /tmp/tc_e.txt /tmp/tc_eo.txt /tmp/tc_e2.txt /tmp/tc_eo2.txt /tmp/tc_s.txt /tmp/tc_so.txt /tmp/tc_m.txt /tmp/tc_mo.txt

# ── tc3 ──
proc hex {s} { binary encode hex $s }
set f [open /tmp/tc_s.txt w]
fconfigure $f -translation binary
puts -nonewline $f [binary format H* c3a9c3a9c3a9]
close $f
foreach src {utf-8 iso8859-1 binary} {
  foreach dst {utf-8 iso8859-1 binary} {
    set s [open /tmp/tc_s.txt r]
    fconfigure $s -translation binary -encoding $src
    set d [open /tmp/tc_d.txt w]
    fconfigure $d -translation binary -encoding $dst
    set n [chan copy $s $d]
    close $s
    close $d
    set f [open /tmp/tc_d.txt r]
    fconfigure $f -translation binary
    set o [read $f]
    close $f
    puts "$src>$dst: n=$n out=[string length $o]B [hex $o]"
  }
}
file delete /tmp/tc_s.txt /tmp/tc_d.txt
