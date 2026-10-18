# __rtcl_http 桥（wasi guest）——阻塞拉式读取循环，事件面与 native 桥同形：
# 逐块 re-entrant eval ::http::_delta（块文本在 $::http_chunk、id 在
# $::http::ev_id）；收尾写 ::http_status / ::http_error / ::http_body 后
# eval ::http::_done_vars。词层 http::*（-stream/-headers/-command/cancel）
# 由 rtcl-vm 的 load_http() 提供（native 契约，见 http.tcl 头注释）。
#
# 与 native 桥的差异：本桥在 guest 内以 Tcl 实现（宿主导入 host::http::* 为
# 拉式原语）；事件源为宿主 reqwest 读取循环，语义（增量、取消、超时收尾）
# 一致。

proc __rtcl_http {id method url body opts headers} {
    # headers: Tcl 列表 → k\tv 行编码（HTTP 头值禁裸 tab/换行，编码无歧义）。
    set enc ""
    foreach {k v} $headers {
        append enc "$k\t$v\n"
    }
    host::http::start $method $url $body $opts $enc
    while {1} {
        set ev [host::http::next]
        set sp [string first " " $ev]
        if {$sp < 0} {
            error "wasi http bridge: malformed event: $ev"
        }
        set type [string range $ev 0 [expr {$sp - 1}]]
        set payload [string range $ev [expr {$sp + 1}] end]
        switch -- $type {
            chunk {
                set ::http_chunk $payload
                set ::http::ev_id $id
                eval ::http::_delta
            }
            done {
                set ::http_status $payload
                set ::http_error ""
                set ::http_body [host::http::body]
                set ::http::ev_id $id
                eval ::http::_done_vars
                return
            }
            error {
                set ::http_status 0
                set ::http_error $payload
                set ::http_body ""
                set ::http::ev_id $id
                eval ::http::_done_vars
                return
            }
            default {
                error "wasi http bridge: unknown event: $type"
            }
        }
    }
}

proc __rtcl_http_cancel {id} {
    host::http::cancel $id
    return
}
