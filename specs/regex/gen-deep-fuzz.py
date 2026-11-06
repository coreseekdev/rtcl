#!/usr/bin/env python3
"""生成嵌套语法层 regexp 随机差分语料（确定性 seed）。

覆盖嵌套分组/非捕获组/环视/量词（含惰性、{m,n}、{,n}）/类/POSIX
类/转义/锚点/内嵌 (?i) 与 -nocase/-line/-linestop/-lineanchor/-all。
pattern/string 以 UTF-8 hex 传入，输出带 D<编号> 标签。

注意：不生成反向引用——tclsh 8.6 的回引回溯在部分随机模式下会
指数级卡死（本语料曾整批超时），回引由 random-4000/semantics/
命令面 fuzz 单独覆盖。

用法:
  python3 specs/regex/gen-deep-fuzz.py [seed] [count] > /tmp/deepfuzz.tcl
  tclsh /tmp/deepfuzz.tcl > o.txt
  ./target/release/rtcl -f /tmp/deepfuzz.tcl > r.txt
  diff o.txt r.txt
"""
import random, sys
seed = int(sys.argv[1]) if len(sys.argv)>1 else 20261113
N = int(sys.argv[2]) if len(sys.argv)>2 else 15000
random.seed(seed)

lits = list("abcABC01_ ") + ["é","中","😀"]
classes = ["[abc]","[^ab]","[a-c]","[[:alpha:]]","[[:digit:]]","[[:space:]]","[.]","[-a]","[a-]","[\\d]","[\\s]","[\\w]"]
escapes = [r"\d",r"\D",r"\s",r"\S",r"\w",r"\W","\\\\",r"\t",r"\n",r"\u0041",r"\x42",r"\101",r"\b",r"\B",r"\y",r"\m",r"\M",r"\Y",r"\A",r"\Z"]
anchors = ["^","$"]

class Gen:
    def __init__(self): self.groups=0
    def atom(self, depth):
        choices = ["lit","class","esc","dot","anchor"]
        if depth>0: choices += ["group","ngroup","look"]
        kind = random.choice(choices)
        if kind=="lit": return random.choice(lits)
        if kind=="class": return random.choice(classes)
        if kind=="esc": return random.choice(escapes)
        if kind=="dot": return "."
        if kind=="anchor": return random.choice(anchors)
        if kind=="group":
            self.groups += 1
            return "(" + self.seq(depth-1) + ")"
        if kind=="ngroup": return "(?:" + self.seq(depth-1) + ")"
        if kind=="look":
            return random.choice(["(?=","(?!"]) + self.seq(depth-1) + ")"
    def quant(self):
        q=random.choice(["","","*","+","?","{1,2}","{0,2}","{2}","{,2}"])
        if q and random.random()<0.25: q+="?"
        return q
    def term(self, depth):
        a=self.atom(depth)
        if a in ("^","$") or a.startswith("(?=(") : return a
        if a[0:2] in ("(?=","(?!"): return a
        return a+self.quant()
    def seq(self, depth):
        n=random.randint(1,3)
        parts=[self.term(depth) for _ in range(n)]
        if random.random()<0.2 and len(parts)>1:
            i=random.randrange(len(parts)); parts=[ "".join(parts[:i])+"|"+"".join(parts[i:]) ]
        return "".join(parts)
    def pat(self):
        p=self.seq(random.randint(0,2))
        if random.random()<0.2: p="(?:)"+p
        return p

texts=["","a","b","ab","abc","a b","a\nb","\na","a\n","aaa","abab","A1_ ","éa","中a","😀a","aé中","123","a\tb","  ","ab\ncd","x"]
flagsets=[[],["-nocase"],["-line"],["-linestop"],["-lineanchor"],["-nocase","-line"],["-all"],["-all","-nocase"],["-all","-line"]]

def hx(s): return s.encode().hex()
print("proc hx {s} {binary decode hex $s}")
for i in range(N):
    g=Gen(); p=g.pat(); s=random.choice(texts); fl=random.choice(flagsets)
    if random.random()<0.05: p="(?i)"+p
    if random.random()<0.03: p="***="+p
    f=" ".join(fl)
    mode=random.choice(["inline","all","vars"])
    if mode=="inline":
        ff=(f+" -inline -indices").strip()
        print(f'''set c [catch {{regexp {ff} -- [hx "{hx(p)}"] [hx "{hx(s)}"]}} r]
if {{$c}} {{puts "D{i} ERR<$r>"}} else {{puts "D{i} OK<$r>"}}''')
    elif mode=="all":
        ff=(f+" -all -inline -indices").strip()
        print(f'''set c [catch {{regexp {ff} -- [hx "{hx(p)}"] [hx "{hx(s)}"]}} r]
if {{$c}} {{puts "D{i} ERR<$r>"}} else {{puts "D{i} OK<$r>"}}''')
    else:
        print(f'''set w SENT; set g1 SENT; set g2 SENT
set c [catch {{regexp {f} -- [hx "{hx(p)}"] [hx "{hx(s)}"] w g1 g2}} err]
if {{$c}} {{puts "D{i} ERR<$err>"}} else {{puts "D{i} OK<[list $w $g1 $g2]>"}}''')
