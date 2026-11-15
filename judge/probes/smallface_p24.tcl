# smallface_p24 — 2026-11-16 组 6/7 小尾巴收口固化探针：
# ::tcl::prefix / ::tcl::Bgerror / ::tcl::CopyDirectory 面（probe1 计数
# 背后）、dict 子命令 errorCode、error 三参 INNER 形、expr 解析错误
# errorCode/INNER、ARITH 首帧关键词、assemble over/label 形、
# getbytecode 装配子集 dict、interp alias {} 形。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|$::errorCode" }

# ::tcl::prefix 面
t pfx1 {::tcl::prefix all {foobar foobaz qux} foo}
t pfx2 {::tcl::prefix longest {foobar foobaz fooqux} foo}
t pfx3 {::tcl::prefix longest {abc ab} a}
t pfx4 {::tcl::prefix match {list length} le}
t pfx5 {::tcl::prefix match -exact {list length} list}
t pfx6 {::tcl::prefix match {list length} l}
t pfx7 {::tcl::prefix zz}
t pfx8 {::tcl::prefix}
t pfx9 {::tcl::prefix all x}
t pfx10 {::tcl::prefix match -message mode {list length} zz}
t pfx11 {::tcl::prefix match -error {A B} {list length} zz}
t pfx12 {::tcl::prefix match -error {} {list length} zz}

# ::tcl::Bgerror 面（无 bgerror handler：tclsh 静默）
t bg1 {::tcl::Bgerror}
t bg2 {::tcl::Bgerror boom}
t bg3 {::tcl::Bgerror boom {}}
t bg4 {::tcl::Bgerror boom {-level 0 -code 3}}

# ::tcl::CopyDirectory 存在性（proc 体文件模式即定义）
puts "cd1|[info commands ::tcl::CopyDirectory]|[info procs ::tcl::CopyDirectory]"

# pkgconfig 面
t pk1 {::tcl::pkgconfig get}
t pk2 {::tcl::pkgconfig zzz x}
t pk3 {llength [::tcl::pkgconfig list]}

# dict 子命令 errorCode
t d1 {dict zzz}
t d2 {set e {k abc}; dict incr e k}
t d3 {dict getdef {a b} zz}

# error 三参 INNER 形（cmd_error returnImm options 列表）
catch {error boom} m1 o1
puts "e1|[dict get $o1 -errorstack]"
catch {error boom ""} m2 o2
puts "e2|[dict get $o2 -errorstack]"
catch {error boom "" {A B}} m3 o3
puts "e3|[dict get $o3 -errorstack]|[dict get $o3 -errorcode]"
catch {error boom "pre" {A B}} m4 o4
puts "e4|[dict get $o4 -errorinfo]|[dict get $o4 -errorstack]"

# expr 解析错误：EC TCL PARSE EXPR * + INNER 形
catch {expr {1 +}} m5 o5
puts "x1|[dict get $o5 -errorcode]|[dict get $o5 -errorstack]"
catch {expr {(}} m6 o6
puts "x2|[dict get $o6 -errorcode]"
catch {expr {)}} m7 o7
puts "x3|[dict get $o7 -errorcode]"
catch {expr {3 #}} m8 o8
puts "x4|[dict get $o8 -errorcode]"
catch {expr {1 lt}} m9 o9
puts "x5|[dict get $o9 -errorcode]"

# ARITH 首帧关键词（双日志事件模型）
catch {expr {1/0}} ma oa
puts "a1|[dict get $oa -errorinfo]|[dict get $oa -errorstack]"

# assemble：over / label 形
t asm1 {tcl::unsupported::assemble {push 1; over 0}}
t asm2 {tcl::unsupported::assemble {push a; push b; swap; pop}}
t asm3 {tcl::unsupported::assemble {jump @nowhere}}
t asm4 {tcl::unsupported::assemble {label @x; label @x}}
t asm5 {tcl::unsupported::assemble {beginCatch @c; push 1; pop; endCatch; push ok; label @c; push bad}}

# getbytecode 装配子集 dict
t gbc1 {tcl::unsupported::getbytecode script {tcl::unsupported::assemble {push 1}}}

# interp alias {} 形（D-1）
t al1 {interp alias {} myal {} puts}
t al2 {myal hello}
t al3 {interp alias {} myal}
t al4 {interp alias nosuch x}
t al5 {interp alias {} nosuchal}
t al6 {interp alias}
