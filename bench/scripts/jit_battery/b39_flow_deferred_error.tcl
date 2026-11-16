# deferred `return -code error`：顶层解码为真错误（消息 = 返回值，
# 帧 = 分派站点；裸消息守卫 ⇒ 首帧 while-executing 形）。
proc p {} {return -code error boom}
p
