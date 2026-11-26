# bn_bits91 — D5 重钉（Tcl 9.1）：位运算大操作数族——& | ^ ~ 与 << >>
# 在 bignum 域的精确性、右移饱和/符号扩展、移位计数边界。
# 9.1.0 oracle 逐字节钉形（2026-11-26）；vm/tree 两引擎一致（sweep）。
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
# 位与/或/异或/取反在 bignum 域
t xor_big {expr {(2**70) ^ 0xff}}
t and_big {expr {(2**70 + 12345) & 0xffff}}
t or_big {expr {(2**70) | 0xff}}
t bnot_big {expr ~(2**70)}
t bnot_big2 {expr ~(2**100 + 1)}
t and_mixed {expr {(2**63+1) & 9223372036854775807}}
t xor_self {expr {(2**70) ^ (2**70)}}
t and_self {expr {(2**70 + 7) & (2**70 + 7)}}
t xor_signext {expr {(-2**70) ^ 0xff}}
t and_neg {expr {(-2**70 - 1) & 0xff}}
t bnot_negbig {expr ~(-2**70)}
# 移位：链式、跨 word 边界、负数右移符号扩展与饱和
t shl_chain {expr {(1<<40) << 40}}
t shl_of_big {expr {(2**70) << 5}}
t shr_big {expr {(2**80) >> 20}}
t shr_neg {expr {(-2**80) >> 20}}
t shr_sat {expr {(2**80) >> 200}}
t shr_sat_neg {expr {(-2**80) >> 200}}
t shr_to_one {expr {(2**70) >> 70}}
t shr_neg2 {expr {(-9223372036854775809) >> 1}}
t shl_then_shr {expr {((2**70) << 10) >> 10}}
# 移位计数面：负计数报错、bignum 计数（正=饱和/负=报错）
t shr_negcnt_big {expr {2**70 >> -1}}
t shl_negcnt {expr {1 << -1}}
t shr_bignum_cnt {expr {(2**80) >> (2**64)}}
t shl_bignum_cnt {expr {1 << 0x8000000000000000}}
# 与比较/逻辑的衔接
t bits_cmp {expr {(2**70) > 2**69}}
t bits_eq {expr {((2**70) | 1) == (2**70 + 1)}}
t bits_truth {expr {(2**64) ? yes : no}}
t bool_big {expr bool(2**64)}
t not_big {expr !9223372036854775808}
