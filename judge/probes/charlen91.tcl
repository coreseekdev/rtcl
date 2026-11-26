# D4 Value 字符长度缓存——一致性与改写链探针（rtcl vs tclsh9.1 逐字节）。
#
# 覆盖三面：
# 1. 非 ASCII（2/3/4 字节混合）串的全字符口径：length/index（正、
#    end-、越界）/range（正、反、越界、end-）/first/last（含显式
#    start/end 索引）/wordend/wordstart；
# 2. 改写链：append（增量维护形）、string replace/map/trim/tolower/
#    toupper/reverse/range 截断（产新值形）、lappend/dict set/incr
#    （rep 变更失效形）、set 变量覆盖、COW 别名（clone 后一边 append）；
# 3. 缓存一致性 fuzz：LCG 确定性伪随机（可重放，非真随机）驱动操作
#    序列，每步前后各问一次长度（命中路径）+ 一次字符口径指纹，oracle
#    有自己的字符缓存——两边逐字节一致 = rtcl 的失效/增量与计数语义
#    全对（rtcl 错缓存会在这里漂移）。
# 等价红线兜底：debug 构建里 Value::char_len 逐调用与重算对拍
# （d4_selfcheck），本探针在 debug 测试跑法下再加一层内部断言。

# ── LCG 确定性伪随机（seed 可重放）──
set seed 20261127
proc rnd {} {
    set ::seed [expr {($::seed * 1103515245 + 12345) & 0x7fffffff}]
    return $::seed
}
proc rpick {lst} {
    return [lindex $lst [expr {[rnd] % [llength $lst]}]]
}

# ── 字符池：1/2/3/4 字节混合 ──
set POOL1 "abcdefghij XYZ_ "
set POOL2 "éüñαΩλ"
set POOL3 "中日한리"
set POOL4 "😀𝕀🦂"

proc rchunk {k} {
    set pools [list $::POOL1 $::POOL2 $::POOL3 $::POOL4]
    set out ""
    for {set i 0} {$i < $k} {incr i} {
        set p [lindex $pools [expr {[rnd] % 4}]]
        append out [string index $p [expr {[rnd] % [string length $p]}]]
    }
    return $out
}

# ── 字符口径指纹：改写链每步的观察面 ──
# （wordstart/wordend 不进指纹：二者在「index 处非单词字符」上的规则
#  rtcl 与 9.1 有**预存**分歧（9.1：wordstart=index / wordend=index+1；
#  rtcl：从 index 起延伸扫描），与 D4 无关，不在本探针钉——ASCII 词内
#  形单独钉在下方静态段。）
proc fp {tag} {
    set s $::s
    puts "$tag L=[string length $s]\
 i0=[string index $s 0] i1=[string index $s 1] i5=[string index $s 5]\
 i9=[string index $s 9] ie0=[string index $s end] ie1=[string index $s end-1]\
 ie3=[string index $s end-3] ie9=[string index $s end-9]\
 ieX=[string index $s end-999] iN=[string index $s -1]\
 r1=[string range $s 2 6] r2=[string range $s end-5 end]\
 r3=[string range $s 0 end-1] r4=[string range $s 7 3]\
 r5=[string range $s end-99 end]\
 f1=[string first é $s] f2=[string first 中 $s] f3=[string first 😀 $s]\
 f4=[string first é $s 3] f5=[string first zz $s]\
 l1=[string last é $s] l2=[string last 中 $s]\
 l3=[string last é $s 2] l4=[string last 中 $s end-3]"
}

# ── 1) 静态多字节串全口径 ──
set s "héllé wörld 中😀_end ünïcode αβγ 日리𝕀"
fp base

# wordstart/wordend 的 ASCII 词内形 + 多字节词内形（index 处都是单词
# 字符——非 index 处规则分歧见上方注记，预存、不在此钉）。
puts "w-ascii ws1=[string wordstart {ab cd_ef} 3] we1=[string wordend {ab cd_ef} 3]\
 ws2=[string wordstart {ab cd_ef} 6] we2=[string wordend {ab cd_ef} 6]\
 ws3=[string wordstart {ab cd_ef} 8] we3=[string wordend {ab cd_ef} 8]\
 w-uni ws4=[string wordstart {ab 中xé} 3] we4=[string wordend {ab 中xé} 3]\
 ws5=[string wordstart {ab 中xé} 5] we5=[string wordend {ab 中xé} 5]"

# ── 2) 改写链（每类入口后长度/索引立即问两遍——命中+一致性）──
set s "é中😀"
append s "xy" "é更多"
fp after-append
set a [string length $s]
set b [string length $s]
puts "twice=$a,$b"

set s [string replace $s 2 4 "ZÜ"]
fp after-replace

set s [string map {é E 中 Z 😀 Q} $s]
fp after-map

set s [string trim "  éZ中  "]
fp after-trim

set s [string tolower "ÉZÜ中😀"]
fp after-lower
set s [string toupper $s 1 3]
fp after-upper-range
set s [string totitle "é中 😀z"]
fp after-title

set s [string reverse "é中😀abc"]
fp after-reverse

set s [string range "0123456789é中😀xyz" 3 end-2]
fp after-range-cut

# rep 变更失效形：list / dict / incr
set s [list a b 中😀]
fp after-list
lappend s é
fp after-lappend
set s [dict create k1 中 k2 😀]
fp after-dict
dict set d k3 é
dict append d k1 é更多
set s $d
fp after-dictset
set n 9
incr n
append n "x"
puts "incr-n=$n len=[string length $n] i1=[string index $n 1]"

# COW 别名形：clone 后只动一边
set base "é中😀xy"
set alias $base
append alias "更多"
puts "cow base=[string length $base] alias=[string length $alias]\
 bi=[string index $base 3] ai=[string index $alias 3]\
 be=[string equal $base "é中😀xy"]"

# 越界/负/错形（错误文案与 errorCode 一并对 oracle 钉）
foreach bad {end+1 99999 -1 x1 1.5 end-x} {
    if {[catch {string index "é中😀xy" $bad} m]} {
        puts "ie-err\[$bad\]=$m|[catch {set ec $::errorCode}]:$ec"
    } else {
        puts "ie-ok\[$bad\]=$m"
    }
    if {[catch {string range "é中😀xy" $bad 2} m]} {
        puts "rg-err\[$bad\]=$m"
    } else {
        puts "rg-ok\[$bad\]=$m"
    }
}

# ── 3) LCG fuzz：确定性伪随机改写链 ×字符口径指纹 ──
set s ""
for {set step 0} {$step < 40} {incr step} {
    set op [rpick {append replace map trim lower upper reverse cut insert}]
    switch -- $op {
        append { append s [rchunk [expr {1 + [rnd] % 6}]] }
        replace {
            set n [string length $s]
            set a [expr {[rnd] % ($n + 1)}]
            set b [expr {$a + [rnd] % 5 - 1}]
            set s [string replace $s $a $b [rchunk [expr {1 + [rnd] % 3}]]]
        }
        map {
            set from [rchunk 1]
            set to [rchunk 1]
            set s [string map [list $from $to] $s]
        }
        trim {
            set s [string trim "[rchunk 2]$s[rchunk 2]"]
        }
        lower {
            set n [string length $s]
            set s [string toupper $s]
            set s [string tolower $s [expr {[rnd] % ($n + 1)}] $n]
        }
        upper {
            set n [string length $s]
            if {$n > 0} {
                set s [string toupper $s [expr {[rnd] % $n}] [expr {$n - 1}]]
            }
        }
        reverse { set s [string reverse $s] }
        cut {
            set n [string length $s]
            if {$n > 2} {
                set s [string range $s [expr {[rnd] % ($n - 1)}] [expr {1 + [rnd] % ($n - 1)}]]
            }
        }
        insert {
            set n [string length $s]
            set s [string replace $s [expr {[rnd] % ($n + 1)}] -1 [rchunk 2]]
        }
    }
    fp f$op-$step
}
puts "fuzz-final L=[string length $s] revL=[string length [string reverse $s]]\
 i-last=[string index $s end] r-half=[string range $s 0 end-1]"
