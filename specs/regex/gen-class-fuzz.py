#!/usr/bin/env python3
"""生成字符类/排序元素/等价类随机差分语料（确定性 seed）。

覆盖 POSIX 类（含 [[:^name:]]）、collating [.x.]、equivalence [=x=]、
范围/否定/类内转义/非 ASCII 类。pattern/text 以 UTF-8 hex 传入
（注意 binary decode 后按字节字符语义，与 tclsh 一致）。

用法:
  python3 specs/regex/gen-class-fuzz.py [seed] [count] > /tmp/classfuzz.tcl
  tclsh /tmp/classfuzz.tcl > o.txt
  ./target/release/rtcl -f /tmp/classfuzz.tcl > r.txt
  diff o.txt r.txt
"""
import random, sys
seed = int(sys.argv[1]) if len(sys.argv) > 1 else 20261118
count = int(sys.argv[2]) if len(sys.argv) > 2 else 4000
random.seed(seed)
classes = ["[[:alpha:]]","[[:^alpha:]]","[[:digit:]]","[[:^digit:]]","[[:space:]]","[[:^space:]]",
           "[[.a.]]","[[.].]]","[[.-.]]","[[=a=]]","[[=é=]]","[[:alpha:]x]","[x[:alpha:]]",
           "[a-c]","[a-]","[-a]","[^a-c]","[à-ÿ]","[中-😀]","[[:alpha:]-z]","[a-[:digit:]]",
           "[\\d]","[\\s]","[\\w]","[.]","[.a.]","[=a=]","[[:foo:]]"]
texts=["","a","A","z","0","9"," ","\t","\n","é","中","😀","abc","a1","-","]","[","=",".","a-c"]
def hx(s): return s.encode().hex()
print("proc hx {s} {binary decode hex $s}")
for i in range(count):
    p=random.choice(classes); s=random.choice(texts)
    if random.random()<0.2: p+=random.choice(["*","+","?","{1,2}"])
    f=random.choice(["-inline -indices","-inline -indices -all","-indices"])
    mode=random.choice(["inline","count"])
    if mode=="inline":
        ff=f+" -inline" if "-inline" not in f else f
        print(f'''set c [catch {{regexp {ff} -- [hx "{hx(p)}"] [hx "{hx(s)}"]}} r]
if {{$c}} {{puts "K{i} ERR<$r>"}} else {{puts "K{i} OK<$r>"}}''')
    else:
        print(f'''set c [catch {{regexp -- [hx "{hx(p)}"] [hx "{hx(s)}"]}} r]
if {{$c}} {{puts "K{i} ERR<$r>"}} else {{puts "K{i} OK<$r>"}}''')
