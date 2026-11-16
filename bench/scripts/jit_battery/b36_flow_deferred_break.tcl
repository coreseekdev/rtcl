# deferred `return -code break` 跨 proc 边界到顶层：顶层解码转错
# （帧 = 分派站点 "p"；wasm kind-1 无循环臂 → 单元 BREAK rc）。
proc p {} {return -code break}
p
