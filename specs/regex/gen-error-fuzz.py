#!/usr/bin/env python3
"""生成 regexp 编译错误码/消息随机差分语料（确定性 seed）。

覆盖括号/方括号/量词/转义/反向引用/内嵌选项/方言前缀的非法组合，
输出 catch 的 rc、消息与 errorCode(REG_*)；pattern 以 hex 传入。

用法:
  python3 specs/regex/gen-error-fuzz.py [seed] [count] > /tmp/errfuzz.tcl
  tclsh /tmp/errfuzz.tcl > o.txt
  ./target/release/rtcl -f /tmp/errfuzz.tcl > r.txt
  diff o.txt r.txt
"""
import random, sys
seed = int(sys.argv[1]) if len(sys.argv) > 1 else 20261121
count = int(sys.argv[2]) if len(sys.argv) > 2 else 10000
random.seed(seed)
alpha=list("ab()[]{}*+?|.^$\\12,-dn<>")+[" "]
def hx(s): return s.encode().hex()
print("proc hx {s} {binary decode hex $s}")
for i in range(count):
    n=random.randint(1,12)
    p="".join(random.choice(alpha) for _ in range(n))
    pref=random.choice(["","(?b)","(?e)","***:","***=","(?i)"])
    p=pref+p
    print(f'''set ::errorCode {{}}
set c [catch {{regexp -- [hx "{hx(p)}"] x}} m]
set e [lindex $::errorCode 1]
if {{$c}} {{puts "X{i} ERR<$m> CODE<$e>"}} else {{puts "X{i} OK"}}''')
