# digits91 — Tcl 9.1 digit-literal grammar probe matrix (T1「数字字面量 9 语法」).
# 9.1 semantics (probed byte-for-byte against tclsh 9.1.0, 2026-11-26):
#   0oNNN/0ONNN = octal (the ONLY octal form); 0d/0D = forced decimal;
#   0b/0B = binary; 0x/0X = hex; bare 0NNN = DECIMAL (legacy octal
#   abolished — 017 is 17, 09 is 9, 018 no longer an invalid octal);
#   underscores may separate digits anywhere inside the digit run of any
#   radix (1_000, 0_1, 1__0 → 10, 0xffff_ffff, 3_14e-1_2) but not at the
#   start, end, or adjacent to a radix prefix (1_, _1, 0x_f, 0o_1 → not
#   numbers). Every integer-accepting entry point shares the grammar:
#   expr, string is integer/wide, incr, lsort -integer/-real, scan %d/%i,
#   int()/wide()/entier().

proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
proc u {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

# --- expr: value of each literal form (braced, so lexing not subst) ---
t dec017 {expr {017}}
t dec09 {expr {09}}
t dec018 {expr {018}}
t dec007 {expr {007}}
t dec00 {expr {00}}
t oct0o17 {expr {0o17}}
t oct0O17 {expr {0O17}}
t oct0o0010 {expr {0o0010}}
t dec0d17 {expr {0d17}}
t dec0D17 {expr {0D17}}
t bin0b101 {expr {0b101}}
t bin0B101 {expr {0B101}}
t hex0x1f {expr {0x1f}}
t hex0X1F {expr {0X1F}}
t us1_000 {expr {1_000}}
t us0_1 {expr {0_1}}
t us1__0 {expr {1__0}}
t us3 {expr {1_2_3}}
t ushex {expr {0xffff_ffff}}
t usbin {expr {0b1111_1110_1101_1011}}
t usoct {expr {0o7_5}}
t usfloat {expr {1_0.5}}
t usexp {expr {3_14e-1_2}}
t usneg {expr {-1_000}}
t usplus {expr {+1_000}}
t neg017 {expr {-017}}
t plus017 {expr {+017}}
t neg09 {expr {-09}}
t neg0o7 {expr {-0o7}}
t plus0o7 {expr {+0o7}}
t neg0d9 {expr {-0d9}}
t plus0d9 {expr {+0d9}}
t neg0x10 {expr {-0x10}}

# --- expr: non-numbers (error shapes) ---
t bad1_ {expr {1_}}
t bad_1 {expr {_1}}
t bad100_ {expr {100_}}
t bad0x_f {expr {0x_f}}
t bad0o_1 {expr {0o_1}}
t bad0d_1 {expr {0d_1}}
t badbare0o {expr {0o}}
t badbare0d {expr {0d}}
t badbare0b {expr {0b}}
t bad0o8 {expr {0o8}}
t bad0d8 {expr {0d8}}
t bad0b2 {expr {0b2}}

# --- expr: eq keeps the raw string form for in-range radix literals ---
t eq017_17 {expr {017 eq 17}}
t eq0o17_15 {expr {0o17 eq 15}}
t eq0d17_17 {expr {0d17 eq 17}}
t eqraw {expr {0x10 eq 0x10}}

# --- expr: unbraced bareword/lexer path ---
u dec017u {expr 017}
u dec09u {expr 09}
u oct0o17u {expr 0o17}
u dec0d17u {expr 0d17}
u us1000u {expr 1_000}
u float085 {expr 08.5}
u float0175 {expr 017.5}
u float08e2 {expr 08e2}
u float0281 {expr 028.1 + 09.2}
u int017 {expr int(017)}
u int09 {expr int(09)}
u wide1000 {expr wide(1_000)}

# --- string is integer / wide ---
t si017 {string is integer -strict 017}
t si09 {string is integer -strict 09}
t si018 {string is integer -strict 018}
t si0o17 {string is integer -strict 0o17}
t si0d17 {string is integer -strict 0d17}
t si1000 {string is integer -strict 1_000}
t si0_1 {string is integer -strict 0_1}
t si1_ {string is integer -strict 1_}
t si1__0 {string is integer -strict 1__0}
t sino7 {string is integer -strict -0o7}
t sip0d9 {string is integer -strict +0d9}
t sib101 {string is integer -strict 0b101}
t six1f {string is integer -strict 0x1f}
t sw017 {string is wide -strict 017}
t sw09 {string is wide -strict 09}
t sw1000 {string is wide -strict 1_000}
t sivalue {string is integer 017}

# --- incr on literal-shaped strings ---
t incr017 {set x 017; incr x; set x}
t incr09 {set x 09; incr x; set x}
t incr1000 {set x 1_000; incr x; set x}
t incr0o17 {set x 0o17; incr x; set x}
t incr0d17 {set x 0d17; incr x; set x}
t incrarg {set x 0; incr x 1_0; set x}
t incrneg017 {set x -017; incr x; set x}

# --- lsort -integer / -real with literal-shaped strings ---
t ls09 {lsort -integer {09 8 017}}
t ls0o {lsort -integer {0o17 8}}
t ls0d {lsort -integer {0d17 8 1_000}}
t lsneg {lsort -integer {-017 9}}
t lsreal {lsort -real {08.5 9 1_0.5}}
t lsbad {lsort -integer {09x 8}}

# --- scan: %d / %i on literal-shaped strings ---
t scd017 {scan 017 %d}
t scd09 {scan 09 %d}
t sci0o17 {scan 0o17 %i}
t sci017 {scan 017 %i}
t scd1000 {scan 1_000 %d}
t sch0x {scan 0x1a %d}
