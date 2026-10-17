# http.tcl —— 数据通道的 wasm 客户端（docs/ROADMAP.md §3）。
#
# 长活在 JS（fetch），Tcl 脚本不被阻塞（浏览器纪律 §4.1/§4.2：禁
# vwait 等返回值）。宿主（页面）用 www/http-bridge.js 注册
# `__rtcl_http` 桥；完成时经 dispatch_event 回灌 ::http::_done。
#
#   http::get  URL -command {…}
#   http::post URL BODY ?-content-type application/json? -command {…}
#
# 回调脚本在全局作用域执行（异步语义，调用栈已散），响应经三个变量
# 读取（Tk % 替身风格，与 ui 的 $ui_row 同族）：
#
#   http_status  整数 HTTP 状态（网络层失败为 0）
#   http_error   错误消息（无错为空串；含超时/网络失败/中止）
#   http_body    响应体文本
#
# 回调跑完三个变量复位。回调脚本自身的错误向上抛（wasm 宿主经
# dispatch_event → bgerror 回路，不静默）。HTTP >= 400 也会回调：
# status 如实、error 为空——由消费方决定语义（curl 同款）。
#
# JSON Lines 助手（摄取格式，ROADMAP §3）：http::jsonl 把响应体切成
# 非空行列表（原始 JSON 串，直接可喂 ui::model set /log/<i>）。

namespace eval ::http {
    variable nextid 0
    variable calls [dict create]   ;# id -> 回调脚本
}

proc ::http::_require_bridge {} {
    if {[llength [info commands __rtcl_http]] == 0} {
        error "http: host bridge __rtcl_http is not registered\
               (wasm 侧用 www/http-bridge.js 安装)"
    }
}

proc ::http::get {url args} {
    return [::http::_request GET $url {} $args]
}

proc ::http::post {url body args} {
    return [::http::_request POST $url $body $args]
}

proc ::http::_request {method url body optargs} {
    if {$url eq ""} { error "http: URL must not be empty" }
    if {[llength $optargs] % 2 != 0} {
        error "http: options must be -name value pairs"
    }
    set command ""
    set timeout 0
    set ctype "application/json"
    foreach {k v} $optargs {
        switch -- $k {
            -command      { set command $v }
            -timeout      { set timeout $v }
            -content-type { set ctype $v }
            default       { error "http: unknown option \"$k\"" }
        }
    }
    if {$command eq ""} {
        # 没有回调 = 错误无处可去（纪律：不许静默）——v1 强制回调。
        error "http: -command is required (完成经回调异步回灌；禁 vwait)"
    }
    ::http::_require_bridge
    variable nextid
    variable calls
    set id [incr nextid]
    dict set calls $id $command
    set opts [dict create timeout $timeout ctype $ctype]
    if {[catch {__rtcl_http $id $method $url $body $opts} err]} {
        dict unset calls $id
        error "http: host bridge failed: $err"
    }
    return $id
}

# 桥的回灌入口：`dispatch_event("::http::_done <id> <status> <err> <body>")`
# （body 由桥做 Tcl 大括号转义）。未知/已完成的 id 静默（迟到事件）。
proc ::http::_done {id status errorMsg body} {
    variable calls
    if {![dict exists $calls $id]} { return }
    set script [dict get $calls $id]
    dict unset calls $id
    set ::http_status $status
    set ::http_error $errorMsg
    set ::http_body $body
    set code [catch {uplevel #0 $script} result]
    unset -nocomplain ::http_status ::http_error ::http_body
    if {$code != 0} {
        return -code $code $result
    }
    return
}

# JSON Lines → 非空行列表（原始 JSON 串）。
proc ::http::jsonl {text} {
    set out [list]
    foreach line [split $text \n] {
        set line [string trim $line]
        if {$line eq ""} { continue }
        lappend out $line
    }
    return $out
}
