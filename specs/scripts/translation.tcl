# -translation 差分脚本：tr1-tr7 合并（set/query/read/gets/unicode/错误形）

# ── tr1 ──
proc hex {s} { binary encode hex $s }
# 写侧：各 translation 下 LF 的落盘字节
foreach mode {lf cr crlf auto platform} {
    set f [open /tmp/rt_tr_w.txt w]
    fconfigure $f -translation $mode
    puts -nonewline $f "a\nb\n"
    close $f
    set f [open /tmp/rt_tr_w.txt r]
    fconfigure $f -translation binary -encoding binary
    set d [read $f]
    close $f
    puts "w-$mode: [hex $d]"
}

# 二元组 {in out}
set f [open /tmp/rt_tr_w2.txt w]
fconfigure $f -translation {auto crlf}
puts -nonewline $f "a\nb\n"
close $f
set f [open /tmp/rt_tr_w2.txt r]
fconfigure $f -translation binary -encoding binary
set d [read $f]
close $f
puts "w-pair: [hex $d]"

# 读侧：同一原始字节在各 translation 下的解码结果
set raw "a\r\nb\rc\nd\r\r\ne"
foreach mode {lf cr crlf auto binary platform} {
    set f [open /tmp/rt_tr_r.txt w]
    fconfigure $f -translation binary
    puts -nonewline $f $raw
    close $f
    set f [open /tmp/rt_tr_r.txt r]
    fconfigure $f -translation $mode -encoding binary
    set d [read $f]
    close $f
    puts "r-$mode: [hex $d]"
}

# 二元组读：{auto lf}
set f [open /tmp/rt_tr_r2.txt w]
fconfigure $f -translation binary
puts -nonewline $f "a\r\nb\rc\nd"
close $f
set f [open /tmp/rt_tr_r2.txt r]
fconfigure $f -translation {auto lf} -encoding binary
set d [read $f]
close $f
puts "r-pair: [hex $d]"

# gets 在 auto 下的切行
set f [open /tmp/rt_tr_r3.txt w]
fconfigure $f -translation binary
puts -nonewline $f "l1\r\nl2\rl3\nl4"
close $f
set f [open /tmp/rt_tr_r3.txt r]
fconfigure $f -translation auto -encoding binary
set out {}
while {[gets $f line] >= 0} { lappend out $line }
close $f
puts "gets-auto: [list $out]"

# read N 跨 CRLF
set f [open /tmp/rt_tr_r4.txt w]
fconfigure $f -translation binary
puts -nonewline $f "a\r\nb"
close $f
set f [open /tmp/rt_tr_r4.txt r]
fconfigure $f -translation auto -encoding binary
puts "read2: [hex [read $f 2]]"
puts "readrest: [hex [read $f]]"
close $f

# 查询形状
set f [open /tmp/rt_tr_q.txt w]
puts "q-default: [fconfigure $f -translation]"
fconfigure $f -translation crlf
puts "q-crlf: [fconfigure $f -translation]"
fconfigure $f -translation {auto crlf}
puts "q-pair: [fconfigure $f -translation]"
fconfigure $f -translation auto
puts "q-auto: [fconfigure $f -translation]"
fconfigure $f -translation binary
puts "q-binary: [fconfigure $f -translation]|[fconfigure $f -encoding]"
fconfigure $f -translation platform
puts "q-platform: [fconfigure $f -translation]"
close $f

# 非法值
set f [open /tmp/rt_tr_q.txt w]
catch {fconfigure $f -translation bogus} m o
puts "bad: $m"
catch {fconfigure $f -translation {auto bogus}} m o
puts "bad2: $m"
catch {fconfigure $f -translation {auto}} m
puts "single-list: $m"
close $f

file delete /tmp/rt_tr_w.txt /tmp/rt_tr_w2.txt /tmp/rt_tr_r.txt /tmp/rt_tr_r2.txt /tmp/rt_tr_r3.txt /tmp/rt_tr_r4.txt /tmp/rt_tr_q.txt

# ── tr2 ──
proc hex {s} { binary encode hex $s }
# 查询形状：各种二元组
foreach pair {{cr lf} {lf crlf} {cr cr} {lf lf} {crlf crlf} {auto lf} {binary binary} {auto crlf}} {
    set f [open /tmp/rt_tr_q2.txt w]
    catch {fconfigure $f -translation $pair} m
    puts "q:$pair -> m=$m q=[fconfigure $f -translation]"
    close $f
}
# 写侧二元组
foreach pair {{cr lf} {lf crlf} {auto cr} {auto lf}} {
    set f [open /tmp/rt_tr_w3.txt w]
    fconfigure $f -translation $pair
    puts -nonewline $f "a\nb\n"
    close $f
    set f [open /tmp/rt_tr_w3.txt r]
    fconfigure $f -translation binary -encoding binary
    set d [read $f]
    close $f
    puts "w:$pair -> [hex $d]"
}
# crlf 输入细格
foreach raw {"\r" "\r\r" "\r\n" "\r\r\n" "\r\n\r\n" "\n\r" "x\r\r\ny"} {
    set f [open /tmp/rt_tr_c.txt w]
    fconfigure $f -translation binary
    puts -nonewline $f $raw
    close $f
    foreach mode {crlf auto cr} {
        set f [open /tmp/rt_tr_c.txt r]
        fconfigure $f -translation $mode -encoding binary
        set d [read $f]
        close $f
        puts "in:$mode raw=[hex $raw] -> [hex $d]"
    }
}
file delete /tmp/rt_tr_q2.txt /tmp/rt_tr_w3.txt /tmp/rt_tr_c.txt

# ── tr3 ──
proc hex {s} { binary encode hex $s }
# unicode(UTF-16LE) 与 translation 的组合：字符层还是字节层？
set f [open /tmp/rt_tr_u.txt w]
fconfigure $f -encoding unicode -translation crlf
puts -nonewline $f "a\nb"
close $f
set f [open /tmp/rt_tr_u.txt r]
fconfigure $f -encoding binary -translation binary
set d [read $f]
close $f
puts "u-write: [hex $d]"

set raw [binary format H* 61000d000a006200]
set f [open /tmp/rt_tr_u2.txt w]
fconfigure $f -translation binary
puts -nonewline $f $raw
close $f
set f [open /tmp/rt_tr_u2.txt r]
fconfigure $f -encoding unicode -translation auto
set d [read $f]
close $f
puts "u-read: [hex $d]"

# 读侧 pair {cr lf} 的实际输入行为
set f [open /tmp/rt_tr_p.txt w]
fconfigure $f -translation binary
puts -nonewline $f "a\r\nb"
close $f
foreach pair {{cr lf} {lf crlf}} {
    set f [open /tmp/rt_tr_p.txt r]
    fconfigure $f -translation $pair -encoding binary
    set d [read $f]
    close $f
    puts "pair-read:$pair -> [hex $d]"
}

# 单元素列表 {auto}
set f [open /tmp/rt_tr_s.txt w]
fconfigure $f -translation {auto}
puts "single-auto-q: [fconfigure $f -translation]"
close $f

# -translation binary 的写侧（LF 是否原样）
set f [open /tmp/rt_tr_b.txt w]
fconfigure $f -translation binary
puts -nonewline $f "a\nb"
close $f
set f [open /tmp/rt_tr_b.txt r]
fconfigure $f -translation binary -encoding binary
set d [read $f]
close $f
puts "binary-write: [hex $d]"

file delete /tmp/rt_tr_u.txt /tmp/rt_tr_u2.txt /tmp/rt_tr_p.txt /tmp/rt_tr_s.txt /tmp/rt_tr_b.txt

# ── tr4 ──
proc hex {s} { binary encode hex $s }
set data "a\r\nb\rc\nd"
set f [open /tmp/rt_tr_g.txt w]
fconfigure $f -translation binary
puts -nonewline $f $data
close $f
# 默认（不设 translation）
set f [open /tmp/rt_tr_g.txt r]
set out {}
while {[gets $f line] >= 0} { lappend out $line }
close $f
puts "gets-default: [list $out]"
# 显式各模式
foreach mode {lf cr crlf auto binary} {
    set f [open /tmp/rt_tr_g.txt r]
    fconfigure $f -translation $mode -encoding binary
    set out {}
    while {[gets $f line] >= 0} { lappend out $line }
    close $f
    puts "gets-$mode: [list $out]"
}
# read N 默认模式
set f [open /tmp/rt_tr_g.txt r]
puts "read2-default: [hex [read $f 2]]"
close $f
file delete /tmp/rt_tr_g.txt

# ── tr5 ──
proc hex {s} { binary encode hex $s }
set raw "a\r\nb"
set f [open /tmp/rt_tr_p2.txt w]
fconfigure $f -translation binary
puts -nonewline $f $raw
close $f
# platform / auto / {auto} 读行为
foreach t {platform auto {auto lf} {auto crlf} {binary lf} {lf binary} {binary binary}} {
    set f [open /tmp/rt_tr_p2.txt r]
    catch {fconfigure $f -translation $t -encoding binary} m
    if {$m eq ""} {
        set d [read $f]
        puts "t:$t q=[fconfigure $f -translation] enc=[fconfigure $f -encoding] read=[hex $d]"
    } else {
        puts "t:$t err=$m"
    }
    close $f
}
# 非法列表
set f [open /tmp/rt_tr_p3.txt w]
foreach t {{} {auto cr lf} {bogus} {lf bogus} {lf lf lf}} {
    catch {fconfigure $f -translation $t} m
    puts "bad:$t -> $m"
}
close $f
# 二进制写侧
foreach t {{binary lf} {lf binary} {binary binary}} {
    set f [open /tmp/rt_tr_p3.txt w]
    catch {fconfigure $f -translation $t; puts -nonewline $f "a\nb"} m
    close $f
    set f [open /tmp/rt_tr_p3.txt r]
    fconfigure $f -translation binary
    set d [read $f]
    close $f
    puts "w:$t -> [hex $d] q? m=$m"
}
file delete /tmp/rt_tr_p2.txt /tmp/rt_tr_p3.txt

# ── tr6 ──
set f [open /tmp/rt_tr_q5.txt w]
puts "1 default: [fconfigure $f -translation]"
fconfigure $f -translation auto
puts "2 auto: [fconfigure $f -translation]"
fconfigure $f -encoding binary
puts "3 enc-binary: [fconfigure $f -translation]|[fconfigure $f -encoding]"
fconfigure $f -translation lf
puts "4 lf: [fconfigure $f -translation]"
fconfigure $f -encoding utf-8
puts "5 enc-utf8: [fconfigure $f -translation]"
fconfigure $f -translation auto
puts "6 auto again: [fconfigure $f -translation]"
fconfigure $f -encoding binary
puts "7 enc-bin again: [fconfigure $f -translation]"
close $f
set f [open /tmp/rt_tr_q5.txt w]
fconfigure $f -encoding binary
puts "8 enc-bin first: [fconfigure $f -translation]"
fconfigure $f -translation auto
puts "9 then auto: [fconfigure $f -translation]"
fconfigure $f -translation {auto crlf}
puts "10 pair: [fconfigure $f -translation]"
fconfigure $f -translation {lf crlf}
puts "11 pair2: [fconfigure $f -translation]"
close $f
file delete /tmp/rt_tr_q5.txt

# ── tr7 ──
set f [open /tmp/rt_tr_z.txt w]
close $f
set f [open /tmp/rt_tr_z.txt r+]
puts "all-rw: [fconfigure $f]"
fconfigure $f -translation {auto crlf}
puts "all-rw2: [fconfigure $f]"
close $f
set f [open /tmp/rt_tr_z.txt r]
puts "all-ro: [fconfigure $f]"
close $f
set f [open /tmp/rt_tr_z.txt w]
puts "all-wo: [fconfigure $f]"
close $f

# binary 元素是否改 encoding
set f [open /tmp/rt_tr_z.txt r]
fconfigure $f -translation {binary lf}
puts "bin-lf: trans=[fconfigure $f -translation] enc=[fconfigure $f -encoding]"
close $f
set f [open /tmp/rt_tr_z.txt w]
fconfigure $f -translation {lf binary}
puts "lf-bin: trans=[fconfigure $f -translation] enc=[fconfigure $f -encoding]"
close $f

# read -nonewline 对未翻译 CRLF
set f [open /tmp/rt_tr_z.txt w]
fconfigure $f -translation binary
puts -nonewline $f "a\r\nb\r\n"
close $f
foreach mode {lf auto} {
    set f [open /tmp/rt_tr_z.txt r]
    fconfigure $f -translation $mode -encoding binary
    puts "nonewline-$mode: [binary encode hex [read -nonewline $f]]"
    close $f
}
file delete /tmp/rt_tr_z.txt
