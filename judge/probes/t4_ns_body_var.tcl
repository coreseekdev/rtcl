# T4 命名空间体裸名变量读写行为探针（vars.rs read_var_scalar_top /
# scalar_top_write_ok / var_slot_fast / get_var 元素读的 ns 前缀快路径
# 改造前钉形）。每行一个断言形，输出与 oracle 逐字节比对；语义敏感区
# （错误文本、errorCode、trace 序、variable 声明、ns delete、数组路由、
# 超长/带 ::/带 ( 名字回落）全覆盖。读/写落点 = T1 的 9.1 规则：裸名只
# 落当前 ns 的表，无全局回落。

proc try {script} {
    if {[catch {uplevel 1 $script} m e] == 0} {
        return "OK:$m"
    }
    return "ERR:$m|code=[dict get [list x [list x $e]] 1 1]?[catch {dict get $e -errorcode} ec]:$ec"
}

# ── 基本读写（命中/未命中/建名落点）──
set g 1
namespace eval n1 { variable v 5 }
namespace eval n1 {
    puts "read=[set v]"
    puts "write=[set v 7]"
    puts "read2=[set v]"
    puts "g-invisible=[catch {set g} m]:$m"
    puts "new-bare=[set q 9]"
    puts "missing=[catch {set nosuch} m]:$m"
    puts "exists=[info exists v],[info exists nosuch],[info exists ::g]"
}
puts "outside=n1::v=[set n1::v],n1::q=[set n1::q],g=[set g]"
puts "g-alive=[info exists ::g]"

# ── variable 声明（有值/无值）──
namespace eval n2 {
    variable w 3
    variable declared
    puts "decl-val=[set w]"
    puts "decl-set=[set declared dv]"
    puts "decl-read=[set declared]"
    puts "decl-miss=[catch {set w2} m]:$m"
}
puts "n2=w:[set n2::w],declared:[set n2::declared],w2:[info exists n2::w2]"

# ── 体里 namespace delete（自删后裸名语义）──
namespace eval n3 { variable k 1 }
namespace eval n3 {
    set k 2
    namespace delete [namespace current]
}
puts "self-del=[namespace exists n3],[catch {set n3::k} m]:$m"

# ── 数组（ns 表宿主 / 元素读写 / is-array 形 / 全局数组不可见）──
set ga(gk) gv
namespace eval n4 {
    variable t
    set t(1) one
    set t(2) two
    array set arr {a x b y}
    puts "arr-read=[set arr(a)]"
    puts "arr-write=[set arr(a) w2]"
    puts "arr-elems=[lsort [array names arr]],[array size arr]"
    puts "arr-miss=[catch {set arr(zz)} m]:$m"
    puts "t-miss=[catch {set t(9)} m]:$m"
    puts "t-new=[set t(3) 3]"
    puts "arr-as-scalar=[catch {set arr} m]:$m"
    set s2 8
    puts "scalar-as-arr=[catch {set s2(k) 1} m]:$m"
    puts "s=[set s2],s-again=[set s2 9]"
    puts "g-arr-invisible=[catch {set ga(gk)} m]:$m"
    puts "incr-elem=[incr t(3)],[incr t(3) 4]"
}
puts "outside4=n4::t(1)=[set n4::t(1)],n4::t(3)=[set n4::t(3)],n4-arr=[set n4::arr(a)],ga=[set ga(gk)]"

# ── incr（裸名：已有/新建/非整数）──
namespace eval n5 {
    set c 10
    puts "incr-having=[incr c],[incr c 5]"
    puts "incr-new=[incr fresh],[incr fresh 2]"
    puts "incr-nonint=[catch {set s x}; catch {incr s} m]:$m"
}
puts "outside5=n5::c=[set n5::c],n5::fresh=[set n5::fresh]"

# ── trace 交互（S9/S14：体里裸名写建 ns 新变量不触全局 trace；
# ns 变量上的 trace 经全链触发——快路径守卫让路）──
set log {}
proc setlog {n v op} { lappend ::log "$op:$n:$v" }
proc rlog {n e op} { lappend ::log "R:$n"; return }
trace add variable g write setlog
namespace eval n6 {
    puts "trace-new=[set g 42]"
    puts "trace-exists-g=[catch {set ::g 43} m]:$m"
    variable tv 1
    puts "ns-read-clean=[set tv]"
}
puts "log6=$::log"
set ::log {}
trace add variable n6::tv write setlog
trace add variable n6::tv read rlog
namespace eval n6 {
    puts "traced-write=[set tv 2]"
    puts "traced-read=[set tv]"
}
puts "log6b=$::log"
trace remove variable n6::tv write setlog
trace remove variable n6::tv read rlog
trace remove variable g write setlog

# ── 限定名与特殊形状（快路径必须让路：::、(、超长）──
namespace eval deep {
    namespace eval inner {
        variable lv 7
        puts "qual=[set lv]"
        puts "rel-qual=[catch {set deep::inner::lv} m]:$m"
        puts "abs=[set ::deep::inner::lv]"
        puts "root-q=[set ::g]"
        puts "root-q-write=[set ::g 99]"
        puts "paren-name=[catch {set a(b} m]:$m"
        set long 1
        puts "long-ok=$long"
    }
}
puts "deep=lv:[set ::deep::inner::lv],g99:[set g]"
namespace eval longns {
    variable veryveryverylongvariablename_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 5
    puts [set veryveryverylongvariablename_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]
    puts [set veryveryverylongvariablename_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 6]
    puts [incr veryveryverylongvariablename_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]
}
puts "longns=[set longns::veryveryverylongvariablename_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa]"

# ── 单冒号名（: 是名字字符——var-1.14）──
namespace eval colon {
    puts "colon-name=[set a:b 1],[set a:b],[catch {set a::b} m]:$m"
}
puts "colon-out=[catch {set colon::a:b} m]:$m[catch {info exists colon::a:b}]"

# ── 体里 upvar/flat 链接（eval 级别名——redirect 路由原样）──
set target old
namespace eval n7 {
    upvar 0 ::target alias
    puts "alias-read=$alias"
    puts "alias-write=[set alias new]"
    puts "alias-incr=[set counter 0; upvar 0 ::counter c2; incr c2; set c2]"
}
puts "n7=target:[set target],counter:[info exists counter],[set ::counter]"

# ── proc 帧与体嵌套（C49：proc 里执行的体有自己的帧；proc 裸名不受扰）──
proc f {} {
    set x proc-local
    namespace eval inner { variable y 1; set y 2 }
    return "$x,[set inner::y]"
}
puts "proc=[f]"

# ── 体里的 info locals（T3 直列不动）。注：体里的 info vars rtcl 现
# 仍列全局（tclsh9.1 只列 ns 自己的表）——预存分歧，不钉（info 面，
# 非 var 读写面；见 T4 施工记录）。
namespace eval n8 { variable a 1; set b 2 }
namespace eval n8 {
    puts "locals=[info locals]"
}

# ── 顶层对照（:: 路径原样）──
set top 1
puts "top=[set top],[set top 2],[incr top],[set ::top]"
puts "done=t4-ns-body-var"
