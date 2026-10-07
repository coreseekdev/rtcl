# bench baseline — bd3e169 2026-09-29
# Linux 7.0.0-31-generic x86_64, tclsh 8.6.17, reps=5 best-of

case               tclsh_ms    rtcl_ms    ratio
arith_loop               43        249      5.7
dict_ops                  8         91     11.3
list_ops                  7        114     16.2
proc_fib                  8         73      9.1
string_build              6          9      1.5
var_incr                 44        194      4.4

# rtcl-data micro baseline — f809dbe 2026-10-07
# Linux 7.0.0-34-generic x86_64, release, best-of-3
# harness: crates/rtcl-data/src/bin/data_bench.rs (lib-level, no interp), fixture bench/fixture.yaml

bench            per-op
yaml_decode_4kb  74.2 us
url_normalize    629 ns
sha256_64b       203 ns
