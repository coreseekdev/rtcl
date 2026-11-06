# regexp ARE 转义语义（\u 1-4 hex、\U 1-8、\x 1-2；\x{} 拒绝；代理区）
# 与 tclsh 8.6.17 差分（braced pattern，不做列表二次反转义）。

proc t {label pat {str A}} {
    set rc [catch {regexp $pat $str} m]
    puts "$label <$pat> rc=$rc m=$m"
}
t u4 {\u0041}
t u3 {\u041}
t u5 {\u00041}
t U8 {\U00000041}
t U4 {\U0041}
t x2 {\x41}
t x1 {\x4}
t xbrace {\x{41}}
t u-nonBMP {\uD800}
t u-zhong {\u4e2d} 中

proc t {pat} {
    set rc [catch {regexp $pat A} m]
    puts "<$pat> rc=$rc m=$m"
}
t {\u}
t {\U}
t {\x}
t {\uZZZZ}
t {\xZZ}
t {\U000000041}
t {[\u0041]}
t {[\uD800]}
