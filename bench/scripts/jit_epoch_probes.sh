#!/usr/bin/env bash
# M3 调查探针：Tier1 命令中途重定义（rename/redefine）对三引擎的观察。
# 发现（2026-10-18）：vm_exec 的 tier1_epoch 只在单元入口检查——
# 同一编译单元内「先 proc set {...} 再 set x 6」时折叠 op 不失效：
#   e1: tclsh/树遍历=5, vm_exec/jit=6（预存解释器分歧，JIT 与 vm_exec 一致）
#   e2: tclsh/树遍历=错误(can't read x), vm_exec/jit=5
#   e3: tclsh=::unknown 无限循环错误, vm_exec/jit=5-7
# 语料不含这些形状（master judge 87/87）。M3 的 JIT 侧 epoch 织入已在
# M1' 落地（入口+回边比对），但 bump 信号源 tier1_epoch 为 rtcl-core
# pub(crate)——JIT 驱动不可见（阻塞点 1）；且消费方 proc-body 执行
# seam 亦被文件互斥阻塞（阻塞点 2）。协调者裁决。
set -u
cd /tmp
printf 'set x 5\nproc set {args} {return 99}\nset x 6\nputs $x\n' > _e1.tcl
printf 'proc set {args} {return 99}\nset x 5\nputs $x\n' > _e2.tcl
printf 'set x 5\nrename set set2\nset y 7\nputs $x-$y\n' > _e3.tcl
RTCL=${RTCL:-/home/nzinfo/src.note/rtcl/target/release/rtcl}
JIT=${JIT:-/home/nzinfo/src.note/rtcl/target/release/jitsh}
for f in _e1 _e2 _e3; do
  echo "== $f =="
  printf "  tclsh    : "; timeout 5 tclsh $f.tcl 2>&1 | head -2
  printf "  tree     : "; timeout 5 "$RTCL" --no-bytecode -f $f.tcl 2>&1 | head -2
  printf "  vm_exec  : "; timeout 5 "$RTCL" -f $f.tcl 2>&1 | head -2
  printf "  jit      : "; timeout 5 "$JIT" -f $f.tcl 2>&1 | head -2
done
