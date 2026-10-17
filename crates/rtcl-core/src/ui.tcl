# ui.tcl — ui::* 词表层（契约见 docs/UI-DSL.md）
#
# 三层设计中的 Tcl 侧：catalog 是单一事实源。词表命令校验参数、维护
# widget/handler/model 注册表，把规范化后的 op（JSON）经唯一的 `_ui`
# 桥命令送到宿主渲染器。`_ui` 由宿主在加载本文件前注册（wasm：JS op
# 汇聚点；原生测试：记录器）。

namespace eval ::ui {
    variable catalog  [dict create]   ;# type -> dict(option -> {kind meta default})
    variable widgets  [dict create]   ;# path -> type
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
    }
}

proc ::ui::_encode_value {kind value} {
    switch -- $kind {
        number  { json encode $value num }
        bool    { json encode $value bool }
        json    { return $value }
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
        if {[lsearch -exact {string script enum number bool json bind size} $kind] < 0} {
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
                if {$name eq "command"} {
                    ::ui::_bind $path click $val
                    if {[lsearch -exact $newev click] < 0} { lappend newev click }
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
proc ::ui::fire {path event} {
    variable handlers
    set key "$path|$event"
    if {![dict exists $handlers $key]} { return "" }
    set result ""
    foreach script [dict get $handlers $key] {
        set result [uplevel 1 $script]
    }
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
    set code [catch {uplevel 1 $script} result]
    set batching 0
    set ops $batchops
    set batchops [list]
    if {$code != 0} {
        set widgets $saved_widgets
        set handlers $saved_handlers
        set events $saved_events
        set model $saved_model
        return -code $code $result
    }
    if {[llength $ops] > 0} {
        _ui "\[[join $ops ,]\]"
    }
    return $result
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
