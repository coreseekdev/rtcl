#!/usr/bin/env python3
"""JIT 差分模糊测试生成器（文法驱动，--seed N 确定性复现）。

语法覆盖 JIT 接受集：名键变量、int 算术/比较/逻辑/位运算、incr、
嵌套 if/elseif/while/for、break/continue、foreach/lmap（多组/{x y} 形）、
lappend、拼接词、[expr] 与 $var 脚本词、proc 形（定义+裸调用）。
约 5% 概率注入 error/nosuchcmd/非数值字符串操作数（锻炼慢路径与错误协议）。

用法: jit_fuzz.py --n 100 --seed 0 [--out DIR]
"""
import argparse
import random
import os

VARS = ["a", "b", "i", "s", "x", "n", "k", "m"]

BIG = [0, -1, 1, 2**31, -(2**31), 2**60, -(2**60), 2**62 - 1, -(2**62), 123456789, -987654321]

ARITH = ["+", "-", "*", "/", "%"]
BIT = ["&", "|", "^", "<<", ">>"]
CMP = ["==", "!=", "<", ">", "<=", ">="]

STRINGS = ["abc", " 42 ", "0x10", "true", "yes", "on", "foo bar", ""]


class Gen:
    def __init__(self, rng: random.Random):
        self.rng = rng
        self.stmts = 0

    def lit(self):
        r = self.rng.random()
        if r < 0.7:
            return str(self.rng.choice(BIG + [self.rng.randint(-100, 100)]))
        if r < 0.8:
            return self.rng.choice(["true", "false", "yes", "no"])
        return f'"{self.rng.choice(STRINGS)}"'

    def operand(self, depth):
        r = self.rng.random()
        if r < 0.5:
            return f"${self.rng.choice(VARS)}"
        if r < 0.8:
            return self.lit()
        if r < 0.9:
            return self.rng.choice(STRINGS)  # 裸词（错误路径）
        return self.expr(depth + 1)

    def expr(self, depth=0):
        """braced expr 词（调用方负责外层 { }）。"""
        if depth > 2:
            return self.operand(depth)
        r = self.rng.random()
        if r < 0.5:
            op = self.rng.choice(ARITH + BIT + CMP)
            return f"{self.operand(depth)} {op} {self.operand(depth)}"
        if r < 0.65:
            op = self.rng.choice(["!", "-", "~"])
            return f"{op}({self.operand(depth)})"
        if r < 0.8:
            op = self.rng.choice(["&&", "||"])
            return f"{self.operand(depth)} {op} {self.operand(depth)}"
        return self.operand(depth)

    def set_stmt(self):
        return f"set {self.rng.choice(VARS)} {self.operand(0)}"

    def incr_stmt(self):
        v = self.rng.choice(VARS)
        if self.rng.random() < 0.5:
            return f"incr {v}"
        return f"incr {v} {self.rng.randint(-10, 10)}"

    def puts_stmt(self):
        r = self.rng.random()
        if r < 0.4:
            return f'puts "${self.rng.choice(VARS)}"'
        if r < 0.7:
            return f"puts [expr {{{self.expr()}}}]"
        if r < 0.85:
            # 拼接词（Concat）
            return f'puts x${self.rng.choice(VARS)}y{self.rng.randint(0, 9)}'
        return f'puts "{self.rng.choice(STRINGS)}"'

    def if_stmt(self, depth):
        body = []
        n = self.rng.randint(1, 2)
        for _ in range(n):
            body.append(self.stmt(depth + 1))
        cond = self.expr()
        out = [f"if {{{cond}}} {{"]
        out += [f"  {b}" for b in body]
        if self.rng.random() < 0.4:
            alt = [self.stmt(depth + 1) for _ in range(self.rng.randint(1, 2))]
            out.append("} else {")
            out += [f"  {a}" for a in alt]
        out.append("}")
        return "\n".join(out)

    def while_stmt(self, depth):
        v = self.rng.choice(VARS)
        guard = self.rng.randint(3, 12)
        body = [self.stmt(depth + 1) for _ in range(self.rng.randint(1, 2))]
        out = [
            f"set {v} 0",
            f"while {{${v} < {guard}}} {{",
            f"  incr {v}",
        ]
        out += [f"  {b}" for b in body]
        if self.rng.random() < 0.25:
            out.append(f"  if {{${v} > {guard - 1}}} break")
        out.append("}")
        return "\n".join(out)

    def for_stmt(self, depth):
        v = self.rng.choice(VARS)
        guard = self.rng.randint(2, 10)
        step = self.rng.choice(["1", "2", "3"])
        body = [self.stmt(depth + 1) for _ in range(self.rng.randint(1, 2))]
        out = [
            f"for {{set {v} 0}} {{${v} < {guard}}} {{incr {v} {step}}} {{",
        ]
        if self.rng.random() < 0.4:
            out.append(f"  if {{${v} == {self.rng.randint(1, guard)}}} continue")
        out += [f"  {b}" for b in body]
        if self.rng.random() < 0.25:
            out.append(f"  if {{${v} >= {guard - 1}}} break")
        out.append("}")
        return "\n".join(out)

    def foreach_stmt(self, depth):
        v = self.rng.choice(VARS)
        shape = self.rng.random()
        if shape < 0.4:
            lst = "{" + " ".join(str(self.rng.randint(-5, 5)) for _ in range(self.rng.randint(0, 5))) + "}"
            head = f"foreach {v} {lst} {{"
        elif shape < 0.6:
            lst = "{" + " ".join(str(self.rng.randint(-5, 5)) for _ in range(self.rng.randint(1, 5))) + "}"
            head = f"foreach {{{v} {self.rng.choice(VARS)}}} {lst} {{"
        elif shape < 0.8:
            l1 = "{" + " ".join(str(self.rng.randint(-5, 5)) for _ in range(self.rng.randint(1, 4))) + "}"
            l2 = "{" + " ".join(str(self.rng.randint(-5, 5)) for _ in range(self.rng.randint(1, 4))) + "}"
            head = f"foreach {v} {l1} {self.rng.choice(VARS)} {l2} {{"
        else:
            head = f"lmap {v} {{1 2 3 4}} {{"
        body = [self.stmt(depth + 1) for _ in range(self.rng.randint(1, 2))]
        if self.rng.random() < 0.3:
            body.append("continue")
        out = [head] + [f"  {b}" for b in body]
        if head.startswith("lmap"):
            out.append(f"  expr {{${v} * 2}}")
        out.append("}")
        return "\n".join(out)

    def lappend_stmt(self):
        return f"lappend {self.rng.choice(VARS)} {self.operand(0)}"

    def proc_stmt(self, depth):
        """proc 定义 + 顶层裸调用（[p …] 括号调用被 Sub/Expr 二分拒收）。"""
        name = self.rng.choice(["p1", "p2", "p3"])
        params = " ".join(self.rng.choice(VARS) for _ in range(self.rng.randint(0, 2)))
        body = [self.stmt(depth + 1) for _ in range(self.rng.randint(1, 3))]
        if self.rng.random() < 0.5:
            body.append(f"return ${self.rng.choice(VARS)}")
        out = [f"proc {name} {{{params}}} {{"]
        out += [f"  {b}" for b in body]
        out.append("}")
        args = " ".join(self.lit() for _ in range(max(1, len(params.split()) if params else 0)))
        out.append(f"{name} {args}".rstrip())
        return "\n".join(out)

    def error_stmt(self):
        r = self.rng.random()
        if r < 0.4:
            return f'error "e{self.stmts}"'
        if r < 0.7:
            return f"nosuchcmd{self.rng.randint(0, 9)} {self.lit()}"
        return f'puts [expr {{"{self.rng.choice(STRINGS)}" + 1}}]'

    def stmt(self, depth=0):
        self.stmts += 1
        if depth > 2:
            return self.set_stmt()
        if self.rng.random() < 0.05:
            return self.error_stmt()
        r = self.rng.random()
        if r < 0.22:
            return self.set_stmt()
        if r < 0.34:
            return self.incr_stmt()
        if r < 0.46:
            return self.puts_stmt()
        if r < 0.56:
            return self.if_stmt(depth)
        if r < 0.64:
            return self.while_stmt(depth)
        if r < 0.72:
            return self.for_stmt(depth)
        if r < 0.82:
            return self.foreach_stmt(depth)
        if r < 0.88:
            return self.lappend_stmt()
        if r < 0.94:
            return self.proc_stmt(depth)
        return f"set {self.rng.choice(VARS)} [expr {{{self.expr()}}}]"

    def unit(self):
        n = self.rng.randint(3, 12)
        return "\n".join(self.stmt(0) for _ in range(n))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=100)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--out", default=None, help="写文件到目录（默认打印到 stdout）")
    args = ap.parse_args()
    for i in range(args.n):
        rng = random.Random(args.seed + i)
        g = Gen(rng)
        text = g.unit()
        if args.out:
            os.makedirs(args.out, exist_ok=True)
            with open(os.path.join(args.out, f"f{args.seed + i:06d}.tcl"), "w") as f:
                f.write(text + "\n")
        else:
            print(f"=== f{args.seed + i:06d} ===")
            print(text)


if __name__ == "__main__":
    main()
