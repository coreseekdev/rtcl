proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t bool_y {expr bool(y)}
t esc374 {expr {"\374" eq [set s \xFC]}}
t shl_big {expr 7244019458077122840<<1}
t pow63 {expr {2**63}}
t pow64 {expr {2**64}}
t pow81 {expr {2**81}}
t pow100 {expr {2**100}}
t hexbig {expr {0x10000000000000000}}
t neghexbig {expr {-0x8000000000000001}}
t shiftnegcnt {expr {-0x8000000000000001 >> 0x8000000000000000}}
t shl64 {expr {1 << 64}}
t shl_neg {expr {1 << -1}}
t shr_neg {expr {16 >> -1}}
t entier22 {expr entier(1e+22)}
t entierInf {expr entier(Inf)}
t entierNeg {expr entier(-1.5)}
t intNeg {expr int(-1.5)}
t int22 {expr int(1e+22)}
t intbig {expr int(9223372036854775808)}
t widebig {expr wide(9223372036854775808)}
t wide22 {expr wide(1e22)}
t widepos {expr wide(1.5)}
t absbig {expr abs(-170141183460469231731687303715884105728)}
t absmin {expr abs(-9223372036854775808)}
t isqrt66 {expr isqrt((1<<66)-1)}
t isqrtNeg {expr isqrt(-4)}
t cmp1 {expr {9223372036854775808 > 9223372036854775807}}
t cmp2 {expr {9223372036854775808 == 9223372036854775808.0}}
t cmp3 {expr {-0x8000000000000001 < -9223372036854775807}}
t modbig {expr {9223372036854775808 % 3}}
t divbig {expr {9223372036854775808 / 3}}
t divneg {expr {-9223372036854775809 / 3}}
t modneg {expr {-9223372036854775809 % 3}}
t modnegdiv {expr {-9223372036854775809 / -3}}
t bnotbig {expr ~9223372036854775808}
t negbig {expr {- 9223372036854775808}}
t andbig {expr {9223372036854775808 & 0xff}}
t orbig {expr {0xff | 9223372036854775808}}
t minbig {expr {min(2**63, 5)}}
t maxbig {expr {max(2**63, 5)}}
t roundbig {expr round(1e22)}
t floor22 {expr floor(1e22)}
t mulbig {expr {9223372036854775807 * 2}}
t addbig {expr {9223372036854775807 + 1}}
t eqstrbig {expr {9223372036854775808 eq 9223372036854775808}}
t bigfloatcmp {expr {2**63 > 1e18}}
t entier_minf {expr entier(-Inf)}
t intofbig {expr int(-9223372036854775808)}
t boolbig {expr bool(2**63)}
t notbig {expr !9223372036854775808}
t repr {::tcl::unsupported::representation 9223372036854775808}
t powexp_too_big3 {expr {3**268435456}}
t powexp_big2 {expr {2**1000}}
