# ui.tcl — ui::* 词表层（契约见 docs/UI-DSL.md）
#
# 三层设计中的 Tcl 侧：catalog 是单一事实源。词表命令校验参数、维护
# widget/handler/model 注册表，把规范化后的 op（JSON）经唯一的 `_ui`
# 桥命令送到宿主渲染器。`_ui` 由宿主在加载本文件前注册（wasm：JS op
# 汇聚点；原生测试：记录器）。

namespace eval ::ui {
    variable catalog  [dict create]   ;# type -> dict(option -> {kind meta default})
    variable widgets  [dict create]   ;# path -> type
    variable props    [dict create]   ;# path -> 规范化 props（快照用，契约 §2.6）
    variable handlers [dict create]   ;# "$path|$event" -> script list
    variable events   [dict create]   ;# path -> 已告知 JS 转发的事件名
    variable model    [dict create]   ;# Tcl 侧模型树
    variable batching 0
    variable batchops [list]
    # 布局选项是所有组件的公共选项（设计见 docs/UI-LAYOUT.md §3），
    # define 时合并进每个组件的 spec；组件 spec 可同名覆盖。
    variable layoutopts [dict create \
        fill    [list enum {none x y both} {}] \
        weight  [list number {} {}] \
        width   [list size {} {}] \
        height  [list size {} {}] \
        align   [list enum {start center end stretch} {}] \
        gap     [list size {} {}] \
        pad     [list size {} {}]]
}

# ── 路径与编码工具 ─────────────────────────────────────────

proc ::ui::_path_check {path} {
    if {![regexp {^\.[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)*$} $path]} {
        error "bad widget path \"$path\": must be .seg.seg, segments \[A-Za-z0-9_-\]"
    }
}

proc ::ui::_parent {path} {
    set i [string last . $path]
    if {$i <= 0} { return "" }
    string range $path 0 [expr {$i - 1}]
}

# 模型值过线：本身能 parse 为 JSON 的原样过（数字/列表/对象），
# 否则按字符串编码。渲染器侧声明自己期望的类型。
# 注意 rtcl 的 json decode 对齐 Tcl json 包、要求根为 object/array，
# 但 RFC 8259 允许裸标量——true/false/null 与 JSON number 同样原样过线，
# 否则 "false" 到 JS 侧真值化出错（!!"false" === true）。
proc ::ui::_json_or_string {value} {
    if {![catch {json decode $value}]} { return $value }
    if {$value eq "true" || $value eq "false" || $value eq "null"} { return $value }
    if {[regexp {^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?$} $value]} { return $value }
    json encode $value str
}

proc ::ui::_check_value {type opt kind meta value} {
    switch -- $kind {
        enum {
            if {[lsearch -exact $meta $value] < 0} {
                error "bad value \"$value\" for $opt on $type: must be one of: [join $meta {, }]"
            }
        }
        number {
            if {![string is double -strict $value]} {
                error "bad value \"$value\" for $opt on $type: expected number"
            }
        }
        bool {
            if {![string is boolean -strict $value]} {
                error "bad value \"$value\" for $opt on $type: expected boolean"
            }
        }
        size {
            if {![regexp {^[0-9]+(\.[0-9]+)?(px|%|em|rem|ch|vw)?$} $value]} {
                error "bad value \"$value\" for $opt on $type: expected size (number, optional px/%/em/rem/ch/vw)"
            }
        }
        json {
            # 原样过桥的 JSON payload（-vegalite 等）：词表层先验语法，
            # 坏了就是 Tcl 错误——别让非法 JSON 到 JS 侧才炸。
            if {[catch {json decode $value}]} {
                error "bad value \"$value\" for $opt on $type: expected JSON"
            }
        }
    }
}

proc ::ui::_encode_value {kind value} {
    switch -- $kind {
        number  { json encode $value num }
        bool    { json encode $value bool }
        json    { return $value }
        list {
            # Tcl 列表 → JSON 数组（-columns 等键名列表；元素各自编码）
            set parts [list]
            foreach e $value { lappend parts [json encode $e str] }
            return "\[[join $parts ,]\]"
        }
        default { json encode $value str }
    }
}

proc ::ui::_encode_events {evlist} {
    set parts [list]
    foreach e $evlist { lappend parts [json encode $e str] }
    return "\[[join $parts ,]\]"
}

proc ::ui::_props_json {type props} {
    variable catalog
    set spec [dict get $catalog $type]
    set parts [list]
    dict for {k v} $props {
        set kind string
        if {[dict exists $spec $k]} { set kind [lindex [dict get $spec $k] 0] }
        if {$k eq "bind"} { set kind string }
        lappend parts "[json encode $k str]:[::ui::_encode_value $kind $v]"
    }
    return "\{[join $parts ,]\}"
}

# ── 桥发射 ────────────────────────────────────────────────

proc ::ui::_emit {opjson} {
    variable batching
    variable batchops
    if {$batching} {
        lappend batchops $opjson
    } else {
        _ui "\[$opjson\]"
    }
}

proc ::ui::_emit_create {type path props evlist} {
    set pj [::ui::_props_json $type $props]
    set ej [::ui::_encode_events $evlist]
    ::ui::_emit "\{\"op\":\"create\",\"path\":[json encode $path str],\"type\":[json encode $type str],\"props\":$pj,\"events\":$ej\}"
}

proc ::ui::_emit_set {path props evlist} {
    set pj [::ui::_props_json [dict get $::ui::widgets $path] $props]
    set ej [::ui::_encode_events $evlist]
    ::ui::_emit "\{\"op\":\"set\",\"path\":[json encode $path str],\"props\":$pj,\"events\":$ej\}"
}

proc ::ui::_emit_delete {path} {
    ::ui::_emit "\{\"op\":\"delete\",\"path\":[json encode $path str]\}"
}

# ── catalog 定义与词表命令生成 ─────────────────────────────

# spec：dict(option -> {kind meta default})；kind ∈
# string|script|enum|number|bool|json|bind。
proc ::ui::define {type spec} {
    variable catalog
    set normspec [dict create]
    dict for {opt entry} $spec {
        if {[llength $entry] != 3} {
            error "ui::define $type: bad spec entry for $opt (need {kind meta default})"
        }
        set kind [lindex $entry 0]
        if {[lsearch -exact {string script enum number bool json bind size list} $kind] < 0} {
            error "ui::define $type: unknown kind \"$kind\" for $opt"
        }
        # catalog 里选项名统一不带前导短横（词表写作 -text，解析键为 text）
        set key $opt
        if {[string match -* $opt]} { set key [string range $opt 1 end] }
        dict set normspec $key $entry
    }
    # 布局是公共选项：合并进组件 spec（组件可同名覆盖）
    dict for {opt entry} $::ui::layoutopts {
        if {![dict exists $normspec $opt]} { dict set normspec $opt $entry }
    }
    dict set catalog $type $normspec
    set lname [string tolower $type]
    # 双引号模板：[list $type] 此刻求值固化类型名；\$path/\$args 保留
    # 到词表命令运行时才替换（不能用 [list] 拼模板——它会给 $ 词加
    # 大括号，运行时变成不替换的字面量）。
    proc ::ui::$lname {path args} "
        ::ui::_create [list $type] \$path \$args
    "
    return
}

# catalog 内省：内部存储的选项名不带前导短横（解析方便），
# 输出时加回 -，展示用户书写的词表形式。
proc ::ui::_catalog_show {spec} {
    set out [dict create]
    dict for {opt entry} $spec {
        dict set out -$opt $entry
    }
    return $out
}

proc ::ui::catalog {{type ""}} {
    variable catalog
    if {$type eq ""} {
        set out [dict create]
        dict for {t spec} $catalog {
            dict set out $t [::ui::_catalog_show $spec]
        }
        return $out
    } elseif {[dict exists $catalog $type]} {
        ::ui::_catalog_show [dict get $catalog $type]
    } else {
        error "unknown component type \"$type\""
    }
}

# ── 选项解析（create/configure 共用）───────────────────────
# 返回 {props newEvents}；-command 等 script 选项留在 Tcl 侧注册，
# JS 只收到"转发此事件"的标记；bind 选项规范化为模型路径。

proc ::ui::_parse_options {type path optvals} {
    variable catalog
    set spec [dict get $catalog $type]
    set props [dict create]
    set newev [list]
    if {[llength $optvals] % 2 != 0} {
        error "$type $path: options must be -name value pairs"
    }
    foreach {opt val} $optvals {
        if {![string match -* $opt]} {
            error "$type $path: expected option (-name value), got \"$opt\""
        }
        set name [string range $opt 1 end]
        if {![dict exists $spec $name]} {
            error "$type $path: unknown option \"$opt\" (have: [join [lsort [dict keys $spec]] {, }])"
        }
        lassign [dict get $spec $name] kind meta default
        ::ui::_check_value $type $opt $kind $meta $val
        switch -- $kind {
            script {
                # script 类选项留在 Tcl 侧注册（Tk -command 语义），JS 只
                # 收到"转发此事件"的标记（events 数组）。
                if {$name eq "command"} {
                    ::ui::_bind $path click $val
                    if {[lsearch -exact $newev click] < 0} { lappend newev click }
                } elseif {$name eq "rowactions"} {
                    # -rowactions {open {…} del {…}}：动作名→脚本 dict。
                    # 脚本不过桥；JS 经 events 数组拿到动作名（渲染器在
                    # 行尾画按钮），点击回灌 fire <path> row<动作> <行号>。
                    if {[llength $val] % 2 != 0} {
                        error "$type $path: -rowactions must be event script pairs"
                    }
                    foreach {k s} $val {
                        if {![regexp {^[A-Za-z][A-Za-z0-9_-]*$} $k]} {
                            error "$type $path: bad rowaction name \"$k\":\
                                   letter first, then letters/digits/_/" \
                                ;# 措辞不带方括号：双引号里 [ ] 是命令替换
                        }
                        ::ui::_bind $path row$k $s
                        if {[lsearch -exact $newev row$k] < 0} { lappend newev row$k }
                    }
                }
                continue
            }
            bind {
                if {$name eq "variable"} { set val "/var/[string trimleft $val :]" }
                dict set props bind $val
                continue
            }
        }
        dict set props $name $val
    }
    list $props $newev
}

# ── 事件绑定与派发 ─────────────────────────────────────────

proc ::ui::_bind {path event script} {
    variable handlers
    variable events
    dict lappend handlers "$path|$event" $script
    if {![dict exists $events $path]} { dict set events $path [list] }
    if {[lsearch -exact [dict get $events $path] $event] < 0} {
        dict lappend events $path $event
    }
}

proc ::ui::on {path event script} {
    variable widgets
    variable events
    if {![dict exists $widgets $path]} { error "unknown widget \"$path\"" }
    set known [expr {[dict exists $events $path]
        && [lsearch -exact [dict get $events $path] $event] >= 0}]
    ::ui::_bind $path $event $script
    if {!$known} {
        ::ui::_emit_set $path [dict create] [dict get $events $path]
    }
    return
}

# JS 事件回灌入口：dispatch_event("::ui::fire .s.b click")。
# 回调脚本在全局作用域执行（Tk 语义）。
# 可选第三参 = 事件的行级参数（ui::table 行事件回灌行号，如
# "::ui::fire .s.t rowopen 42"）：rowaction/-command 一类脚本经调用方
# 作用域里的 $ui_row 读取（Tk % 替换的轻量替身——v1 不做通用
# 替换机制）。无 detail 的 fire 不设该变量；回调结束即清理。
proc ::ui::fire {path event {detail ""}} {
    variable handlers
    set key "$path|$event"
    if {![dict exists $handlers $key]} { return "" }
    set result ""
    if {$detail ne ""} { uplevel 1 [list set ui_row $detail] }
    foreach script [dict get $handlers $key] {
        set result [uplevel 1 $script]
    }
    if {$detail ne ""} { uplevel 1 [list unset -nocomplain ui_row] }
    return $result
}

# ── 词表命令的公共实现 ─────────────────────────────────────

proc ::ui::_create {type path arglist} {
    variable catalog
    variable widgets
    variable events
    ::ui::_path_check $path
    if {[dict exists $widgets $path]} {
        error "widget \"$path\" already exists"
    }
    if {$type ne "Surface"} {
        set parent [::ui::_parent $path]
        if {$parent eq "" || ![dict exists $widgets $parent]} {
            error "widget \"$path\": parent \"$parent\" does not exist"
        }
    }
    lassign [::ui::_parse_options $type $path $arglist] props newev
    # 非空默认值补上
    dict for {opt entry} [dict get $catalog $type] {
        lassign $entry kind meta default
        if {$default ne "" && ![dict exists $props $opt]} {
            ::ui::_check_value $type -$opt $kind $meta $default
            dict set props $opt $default
        }
    }
    dict set widgets $path $type
    dict set ::ui::props $path $props
    if {![dict exists $events $path]} { dict set events $path [list] }
    foreach e $newev {
        if {[lsearch -exact [dict get $events $path] $e] < 0} {
            dict lappend events $path $e
        }
    }
    ::ui::_emit_create $type $path $props [dict get $events $path]
    return $path
}

proc ::ui::configure {path args} {
    variable widgets
    variable events
    if {![dict exists $widgets $path]} { error "unknown widget \"$path\"" }
    set evlist [list]
    if {[dict exists $events $path]} { set evlist [dict get $events $path] }
    lassign [::ui::_parse_options [dict get $widgets $path] $path $args] props newev
    # 快照注册表合并（契约 §2.6：snapshot 从注册表反生成）
    set cur [dict create]
    if {[dict exists $::ui::props $path]} { set cur [dict get $::ui::props $path] }
    dict for {k v} $props { dict set cur $k $v }
    dict set ::ui::props $path $cur
    ::ui::_emit_set $path $props $evlist
    return $path
}

proc ::ui::destroy {path} {
    variable widgets
    variable handlers
    variable events
    if {![dict exists $widgets $path]} { error "unknown widget \"$path\"" }
    set doomed [list]
    dict for {p t} $widgets {
        if {$p eq $path || [string match "$path.*" $p]} { lappend doomed $p }
    }
    foreach p $doomed {
        dict unset widgets $p
        if {[dict exists $events $p]} { dict unset events $p }
        if {[dict exists $::ui::props $p]} { dict unset ::ui::props $p }
        dict for {k v} $handlers {
            if {[string match "$p|*" $k]} { dict unset handlers $k }
        }
    }
    ::ui::_emit_delete $path
    return
}

# 构建事务：批次内的 op 攒成一次过桥；出错则整个批次作废。
# 作废不止是不发 ops——批内已登记的 widget/handler/model 也要回滚，
# 否则 JS 没收到 create 而 Tcl 侧以为已建，两侧状态分裂。
proc ::ui::batch {script} {
    variable batching
    variable batchops
    variable widgets
    variable handlers
    variable events
    variable model
    if {$batching} { error "ui::batch cannot be nested" }
    set batching 1
    set batchops [list]
    set saved_widgets $widgets
    set saved_handlers $handlers
    set saved_events $events
    set saved_model $model
    set saved_props $::ui::props
    set code [catch {uplevel 1 $script} result]
    set batching 0
    set ops $batchops
    set batchops [list]
    if {$code != 0} {
        set widgets $saved_widgets
        set handlers $saved_handlers
        set events $saved_events
        set model $saved_model
        set ::ui::props $saved_props
        return -code $code $result
    }
    if {[llength $ops] > 0} {
        _ui "\[[join $ops ,]\]"
    }
    return $result
}

# ── 快照（契约 §2.6）──────────────────────────────────────
# 从注册表反生成等价构建脚本：snapshot 是纯函数（当前状态 → 脚本），
# snapshot + 后续 delta ≡ 完整历史。

# 模型树递归展开为 {path value} 叶子对（node 若非 dict 视为叶子）
proc ::ui::_model_dump {node prefix outVar} {
    upvar $outVar acc
    if {[catch {dict size $node} n] || $n == 0} {
        lappend acc [list $prefix $node]
        return
    }
    dict for {k v} $node {
        ::ui::_model_dump $v "$prefix/$k" acc
    }
}

proc ::ui::snapshot {path args} {
    variable widgets
    variable props
    variable handlers
    if {![dict exists $widgets $path]} { error "unknown widget \"$path\"" }
    set modelPrefix ""
    foreach {k v} $args {
        if {$k eq "-model"} { set modelPrefix $v } else {
            error "ui::snapshot: unknown option \"$k\""
        }
    }
    set lines [list]
    dict for {p t} $widgets {
        if {!($p eq $path || [string match "$path.*" $p])} { continue }
        set cmd [list ::ui::[string tolower $t] $p]
        if {[dict exists $props $p]} {
            dict for {k v} [dict get $props $p] {
                # bind 重建为 -path（-variable 糖不还原，契约 §2.6）
                if {$k eq "bind"} { lappend cmd -path $v } else { lappend cmd -$k $v }
            }
        }
        # 回调：首个并入创建行 -command，其余重建 ui::on 行
        set extra [list]
        dict for {key scripts} $handlers {
            if {![string match "$p|*" $key]} { continue }
            set ev [string range $key [expr {[string length $p] + 1}] end]
            set first 1
            foreach s $scripts {
                if {$first && $ev eq "click"} {
                    lappend cmd -command $s
                } else {
                    lappend extra [list ::ui::on $p $ev $s]
                }
                set first 0
            }
        }
        lappend lines $cmd
        foreach e $extra { lappend lines $e }
    }
    if {$modelPrefix ne ""} {
        set leaves [list]
        set keys [::ui::_model_keys $modelPrefix]
        if {[dict exists $::ui::model {*}$keys]} {
            ::ui::_model_dump [dict get $::ui::model {*}$keys] $modelPrefix leaves
        }
        foreach pair $leaves {
            lassign $pair mp mv
            lappend lines [list ::ui::model set $mp $mv]
        }
    }
    join $lines \n
}

# ── 模型域 ────────────────────────────────────────────────

proc ::ui::_model_keys {path} {
    set p [string trimleft $path /]
    if {$p eq ""} { error "ui::model: bad path \"$path\"" }
    split $p /
}

proc ::ui::model {sub args} {
    variable model
    switch -- $sub {
        set {
            if {[llength $args] != 2} { error "usage: ui::model set path value" }
            lassign $args path value
            dict set model {*}[::ui::_model_keys $path] $value
            ::ui::_emit "\{\"op\":\"model\",\"sub\":\"set\",\"path\":[json encode $path str],\"value\":[::ui::_json_or_string $value]\}"
            return $value
        }
        get {
            if {[llength $args] != 1} { error "usage: ui::model get path" }
            set path [lindex $args 0]
            set keys [::ui::_model_keys $path]
            if {![dict exists $model {*}$keys]} {
                error "ui::model: no such path \"$path\""
            }
            dict get $model {*}$keys
        }
        unset {
            if {[llength $args] != 1} { error "usage: ui::model unset path" }
            set path [lindex $args 0]
            set keys [::ui::_model_keys $path]
            if {[dict exists $model {*}$keys]} { dict unset model {*}$keys }
            ::ui::_emit "\{\"op\":\"model\",\"sub\":\"unset\",\"path\":[json encode $path str]\}"
            return
        }
        default {
            error "ui::model: unknown subcommand \"$sub\": must be set, get, unset"
        }
    }
}

# ── 基础 catalog（L0；L1 分子组件随 JS 渲染器落地再注册）────

::ui::define Surface {
    -title {string {} {}}
}
::ui::define Row {}
::ui::define Column {}
::ui::define Card {}
::ui::define Divider {}
::ui::define Text {
    -text    {string {} {}}
    -variant {enum {h1 h2 h3 caption body} {}}
}
::ui::define Button {
    -text    {string {} {}}
    -variant {enum {primary borderless} {}}
    -command {script {} {}}
}
::ui::define Field {
    -label    {string {} {}}
    -path     {bind {} {}}
    -variable {bind {} {}}
}
::ui::define Checkbox {
    -label {string {} {}}
    -path  {bind {} {}}
}

# ── L1 分子组件（随 JS 渲染器逐个落地；渲染器见 ui-runtime.js）──

# 行数组表格。-path 是 bind 类选项：指向模型里的行数组（契约 §2.3；
# 数据喂数走 ui::model set 子路径增量，禁一次性全量 JSON——
# UI-RESEARCH §2.1/§5）。就和脚本里其它 dict 值一样：
#
#   ui::model set /pods/42 [format {{"name":"pod-%d","status":"Running"}} 42]
#   ui::model set /pods/108 {{"name":"pod-108","status":"Pending"}}
#
# Tcl 引词规则：一层 {…} 在传参时就去壳，字面量行 JSON 要双大括号
# （或运行时字符串）；不要预置 /pods 标量（dict set 自动建父——与
# tclsh 一致，标量再下钻报 "missing value to go with key"）。行在
# 模型树里存为原始 JSON 串：v1 只支持整行增量；cell 级更新需行值为
# dict（模型通道演进，docs/UI-DSL.md 变更建议 §8）。
#
# -columns 为列键的 Tcl 列表（list kind → JSON 数组过线）。
# -rowactions {open {…}} 的动作脚本留在 Tcl 侧（Tk -command 同款），
# JS 只收 row<动作> 转发标记并在行尾画按钮；行点击回灌 rowclick
# （默认无 handler，经 ui::on 绑）。行级动作脚本由把行号放在
# $ui_row（见 ::ui::fire 注释）。
::ui::define Table {
    -path       {bind {} {}}
    -columns    {list {} {}}
    -rowactions {script {} {}}
}

# 日志流（L1 第二个分子组件；UI-RESEARCH §3.2）。-path 是 bind 类
# 选项：模型里的行数组，**追加式喂数**（与 Table 同款游标）：
#
#   ui::model set /deploylog/0 [format {{"ts":"00:00:01","level":"info","text":"ok"}}]
#   ui::model set /deploylog/1 {{"level":"warn","text":"slow step"}}
#
# 行值 JSON 对象的 level 字段决定颜色类（info/warn/error/debug …，
# 样式在渲染器 CSS；未知级别落默认样式）；标量行值视为 info。
# 正则高亮 v1 不做（docs/UI-DSL.md 变更建议 §8）。行点击回灌
# lineclick（ui::on 绑定，行号放 $ui_row——见 ::ui::fire 注释）；
# 无动作按钮（行动作是 Table 的事，log 的钻取走 lineclick →
# 新 surface）。
::ui::define Log {
    -path {bind {} {}}
}

# 键值详情（L1 详情双件套之一；UI-RESEARCH §2.3/§3）。-path 是 bind
# 类选项：模型里的一个对象，渲染为"键 | 值"两列（标量原样，嵌套
# 对象/数组 compact JSON 单行——要下钻用 ui::tree）。键序默认 = 模型
# 对象的插入序；-keys 给显式键序（Tcl 列表 → JSON 数组）只渲染列出的
# 键（缺失的跳过）。子路径增量（ui::model set /info/status X）原位
# 更新对应行，不重建其它行。
::ui::define Kv {
    -path {bind {} {}}
    -keys {list {} {}}
}

# JSON 树（详情双件套之二）。-path 指向任意 JSON 值：对象/数组按
# 键/下标递归，容器节点默认折叠（预览 "{…} N keys" / "[…] N items"），
# 点击逐层展开——DOM 只物化可见节点（"懒加载子节点" = 渲染惰性，
# 数据本就在模型里）。标量节点单行显示。模型子树更新后按当前展开
# 状态重绘可见节点。
::ui::define Tree {
    -path {bind {} {}}
}

# 图表（L1 数据分析组件；UI-RESEARCH §2.2/§2.3）。-vegalite 是
# JSON 类选项：payload 即 Vega-Lite spec（v1 支持子集），原样过桥。
# 数据走 model 通道（图表定义与数据分离）：spec 的
# "data":{"name":"<模型路径>"} 声明数据起点——行数组，游标喂数同
# Table/Log；渲染器订阅该前缀，增量 append 只重绘图形（rAF 合并），
# spec 不变不重建组件。
#
#   ui::chart .c -vegalite {
#     {"data": {"name": "/cpu"},
#      "mark": "line",
#      "encoding": {"x": {"field": "t"}, "y": {"field": "v", "type": "quantitative"}}}
#   }
#   （一层 {…} 在传参时就被剥掉，JSON 外层大括号要写在引词花括号里面
#    ——与 model 行 JSON 的双大括号规则同源。）
#
# v1 支持 mark: line/bar/point/area；x 一律按序数等距摆放（时序
# 标签原样显示）；完整 Vega-Lite 语法需引入库（docs/UI-DSL.md §8）。
::ui::define Chart {
    -vegalite {json {} {}}
}

# 分布式 trace 瀑布（L1 渲染新原语；UI-RESEARCH §3.1）。-path 是
# bind 类选项：span 树 {name, start_ms, dur_ms, children[]}（也接受
# span 数组 = 多根）。甘特式时间轴：每 span 一行，条的位置/宽度按
# 完整树的全局 [min start, max end] 域百分比摆放——折叠子树不改变
# 对齐；点击名字/箭头折叠（默认全展开），点击条缩放到该 span 的时
# 间窗（再点复位），点轨道空白复位缩放。折叠/缩放纯客户端，不触
# Tcl。树里缺数值 start_ms/dur_ms 的 span 防御性跳过（数据质量归
# 喂数侧，UI-DSL.md §8 条目 13）。
::ui::define Trace {
    -path {bind {} {}}
}
