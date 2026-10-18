# http.tcl —— HTTP 词表层（native / wasm 事件面同形）。
#
#   http::get  URL ?-command {…}? ?-stream {…}? ?-timeout ms? ?-content-type type?
#   http::post URL BODY ?同上?
#
# -stream 脚本：每收到一块原始字节执行一次；块文本在 $::http_chunk，
#   回调期间 $::http_id 为本次请求 id。适合 SSE / LLM 流式逐块消费。
# -command 脚本：完成后执行一次，响应经三个变量读取（v1 契约不变）：
#   http_status  整数 HTTP 状态（网络层失败/中止为 0）
#   http_error   错误消息（无错为空串；含超时/网络失败/取消）
#   http_body    响应体完整文本
#
# 宿主桥（二选一，Tcl 侧无感）：
#   · wasm：www/http-bridge.js 注册 __rtcl_http；完成经 dispatch_event 回灌
#     ::http::_done。
#   · native：rtcl-cli `net` feature 注册 __rtcl_http（阻塞读取循环，
#     逐块 re-entrant eval ::http::_delta，收尾 ::http::_done_vars）
#     与 __rtcl_http_cancel。
#
# 取消：http::cancel id —— 读取循环在下一块边界协作中止，随后 _command
# 以 (status 0, error cancelled) 收尾。
#
# 回调脚本在全局作用域执行（异步语义，调用栈已散）；错误向上抛，读取
# 变量在回调后复位。HTTP >= 400 也回调（status 如实、error 为空——
# curl 同款，语义归消费方）。
#
# JSON Lines 助手（摄取格式，ROADMAP §3）：http::jsonl 把响应体切成
# 非空行列表（原始 JSON 串，直接可喂 ui::model set /log/<i>）。

namespace eval ::http {
    variable nextid 0
    variable calls [dict create]   ;# id -> {command stream}
}

proc ::http::_require_bridge {} {
    if {[llength [info commands __rtcl_http]] == 0} {
        error "http: host bridge __rtcl_http is not registered\
               (wasm 侧用 www/http-bridge.js 安装；native 侧用 --features net 构建)"
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
    set stream ""
    set timeout 0
    set ctype "application/json"
    foreach {k v} $optargs {
        switch -- $k {
            -command      { set command $v }
            -stream       { set stream $v }
            -timeout      { set timeout $v }
            -content-type { set ctype $v }
            default       { error "http: unknown option \"$k\"" }
        }
    }
    if {$command eq "" && $stream eq ""} {
        # 没有回调 = 错误无处可去（纪律：不许静默）——v1 强制回调。
        error "http: -command is required (完成经回调异步回灌；禁 vwait)"
    }
    if {![string is integer -strict $timeout] || $timeout < 0} {
        error "http: -timeout must be a non-negative integer (ms)"
    }
    ::http::_require_bridge
    variable nextid
    variable calls
    set id [incr nextid]
    dict set calls $id [list $command $stream]
    # 选项以 JSON 过桥（JS 侧 JSON.parse；手搓小对象——json encode
    # 无 schema 时按字符串编码，见 ui.tcl 同款做法）。-stream 仅在
    # 需要时出现，保持既有线格式不变。
    set opts "\{\"timeout\":$timeout,\"ctype\":[json encode $ctype str]"
    if {$stream ne ""} { append opts ",\"stream\":true" }
    append opts "\}"
    if {[catch {__rtcl_http $id $method $url $body $opts} err]} {
        dict unset calls $id
        error "http: host bridge failed: $err"
    }
    return $id
}

# -stream 回调入口（native 桥逐块调用）：块文本在 $::http_chunk，
# 请求 id 在 $::http::ev_id（宿主 set_var 注入，免转义）。
proc ::http::_delta {} {
    variable calls
    variable ev_id
    if {![info exists ev_id]} { return }
    set id $ev_id
    unset ev_id
    if {![dict exists $calls $id]} {
        unset -nocomplain ::http_chunk
        return
    }
    set stream [lindex [dict get $calls $id] 1]
    if {$stream eq ""} {
        unset -nocomplain ::http_chunk
        return
    }
    set ::http_id $id
    set code [catch {uplevel #0 $stream} result]
    unset -nocomplain ::http_id ::http_chunk
    if {$code != 0} {
        return -code $code $result
    }
    return
}

# 桥的回灌入口（wasm / 旧式）：`dispatch_event("::http::_done <id> <status> <err> <body>")`
# （body 由桥做 Tcl 大括号转义）。未知/已完成的 id 静默（迟到事件）。
proc ::http::_done {id status errorMsg body} {
    set ::http_status $status
    set ::http_error $errorMsg
    set ::http_body $body
    ::http::_complete $id
}

# native 桥的回灌入口：status/error/body 已由宿主 set_var 写入（免转义），
# id 在 $::http::ev_id。
proc ::http::_done_vars {} {
    variable ev_id
    if {![info exists ev_id]} { return }
    set id $ev_id
    unset ev_id
    ::http::_complete $id
}

# 完成收尾：跑 -command 回调，随后复位读取变量；未登记 id 静默。
proc ::http::_complete {id} {
    variable calls
    if {![dict exists $calls $id]} {
        unset -nocomplain ::http_status ::http_error ::http_body
        return
    }
    set entry [dict get $calls $id]
    dict unset calls $id
    set command [lindex $entry 0]
    set code 0
    set result ""
    if {$command ne ""} {
        set ::http_id $id
        set code [catch {uplevel #0 $command} result]
        unset -nocomplain ::http_id
    }
    unset -nocomplain ::http_status ::http_error ::http_body
    if {$code != 0} {
        return -code $code $result
    }
    return
}

# 协作取消（native 桥读取循环在块边界检查；wasm 桥未实现前报错，不静默）。
proc ::http::cancel {id} {
    if {[llength [info commands __rtcl_http_cancel]] == 0} {
        error "http: cancel is not supported by this host bridge"
    }
    __rtcl_http_cancel $id
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
