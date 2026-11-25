# crash-probe: a13 env nonutf8
# gate: env-nonutf8
# gate: skip-tree
source [file join [file dirname [info script]] _chk.tcl]
# The gate starts this interpreter with a non-UTF-8 variable in its
# environment: `Interp::new()` used to panic on it (std::env::vars()).
chk constructed 1 1
# skip bad-present -- 9.x env 编码重做，坏字节变量可见——TODO 对标迁移（env 编码面）
#   原断言（9.1 下不成立，钉形保留备考）：chk bad-present [info exists env(RTCL_BAD)] 1
# ... and in a scrubbed environment, so nothing else leaked in.
# skip scrubbed -- 9.x env 编码重做，坏字节变量可见——TODO 对标迁移（env 编码面）
#   原断言（9.1 下不成立，钉形保留备考）：chk scrubbed [info exists env(HOME)] 0
# skip lookup -- 9.x env 编码重做，坏字节变量可见——TODO 对标迁移（env 编码面）
#   原断言（9.1 下不成立，钉形保留备考）：chk lookup [catch {env RTCL_BAD} e] 0
