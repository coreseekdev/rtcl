# C33 probe: `file attributes`（9.1 unix 的 -group/-owner/-permissions）、
# `file tildeexpand`（9.0 新命令，路径名里的 ~ 不再隐式展开）、
# `file home ?user?`、`file normalize` 的 ~ 语义（9.0 起 ~ 是普通
# 分量）。期望值全部按 tclsh 9.1.0 实测（tclUnixFCmd.c 的属性表与
# tclPathObj.c 的 Tcl_FSTildeExpand），环境相关值（HOME、用户/组名、
# 工作目录）在脚本内取值，两边进程同环境，逐字节可比。
set ::errorCode NONE

# catch 的三元组：完成码、消息、errorCode
proc T {script} {
    set c [catch {uplevel 1 $script} r]
    return [list $c $r $::errorCode]
}

# 环境锚：HOME、当前用户/组、工作目录、临时文件（固定在 /tmp 下）
set home $env(HOME)
set cwd [pwd]
set f /tmp/c33attrs.txt
catch {file delete $f}
set fh [open $f w]
close $fh
set uname [file attributes $f -owner]
set gname [file attributes $f -group]

# ── attributes：取值形 ────────────────────────────────────────────────
puts "all-key=[lmap {k v} [file attributes $f] {set k}]"
puts "perm-dir=[file attributes /tmp -permissions]"
puts "T-missing-one=[T {file attributes /nonexistent/xyz -permissions}]"
puts "T-missing-all=[T {file attributes /nonexistent/xyz}]"
puts "T-badopt=[T {file attributes /tmp -bogus}]"
puts "T-set-tmp=[T {file attributes /tmp -permissions 0777}]"
puts "T-noargs=[T file\ attributes]"
puts "T-owner-consumes-next=[T {file attributes /tmp -owner -group}]"
puts "attr-owner-name=$uname"
puts "attr-group-name=$gname"

# ── attributes：设置形（数字、八进制、十六进制、rwx、chmod 串）───────
puts "T-set-oct=[T {file attributes $f -permissions 0741}]"
puts "get-oct=[file attributes $f -permissions]"
puts "T-set-dec=[T {file attributes $f -permissions 493}]"
puts "get-dec=[file attributes $f -permissions]"
puts "T-set-hex=[T {file attributes $f -permissions 0x1a4}]"
puts "get-hex=[file attributes $f -permissions]"
puts "T-set-rwx=[T {file attributes $f -permissions rwx------}]"
puts "get-rwx=[file attributes $f -permissions]"
puts "T-set-ugo=[T {file attributes $f -permissions ug=rw}]"
puts "get-ugo=[file attributes $f -permissions]"
puts "T-set-list=[T {file attributes $f -permissions u+x,go-w}]"
puts "get-list=[file attributes $f -permissions]"
puts "T-set-minus=[T {file attributes $f -permissions o-r}]"
puts "get-minus=[file attributes $f -permissions]"
puts "T-set-bad=[T {file attributes $f -permissions notamode}]"
puts "T-set-badoct=[T {file attributes $f -permissions 018}]"
puts "get-badoct=[file attributes $f -permissions]"
puts "T-set-nouser=[T {file attributes $f -owner nosuchuserxyz}]"
puts "T-set-nogroup=[T {file attributes $f -group nosuchgroupxyz}]"

# ── tildeexpand ──────────────────────────────────────────────────────
puts "te-self-sub=[file tildeexpand ~/x/y]=$home/x/y"
puts "te-self=[file tildeexpand ~]=$home"
puts "te-plain=[file tildeexpand plain/path]"
puts "te-midtilde=[file tildeexpand a~b]"
puts "te-root=[file tildeexpand ~root/x]"
puts "T-te-nouser=[T {file tildeexpand ~nosuchuserabcxyz/x}]"
puts "T-te-tildetilde=[T {file tildeexpand ~~root/x}]"
puts "T-te-noargs=[T file\ tildeexpand]"
puts "T-te-2args=[T {file tildeexpand a b}]"
# （HOMELESS 形——unset env(HOME) 后 `~/x` 报
# `couldn't find HOME environment variable to expand path` +
# TCL VALUE PATH HOMELESS——在 tclsh 上实测过；不进本探针：rtcl 的
# $env 数组 unset 尚不落进程环境，两边环境不同不可比。）

# ── home ─────────────────────────────────────────────────────────────
puts "home=[file home]=$home"
puts "home-root=[file home root]"
puts "T-home-nouser=[T {file home nosuchuserabcxyz}]"
puts "T-home-2args=[T {file home root extra}]"

# ── normalize：~ 不再展开（9.0 起），词法规范化不变 ───────────────────
puts "norm-tilde=[file normalize ~]=$cwd/~"
puts "norm-tilde-sub=[file normalize ~/sub]=$cwd/~/sub"
puts "norm-tilde-user=[file normalize ~nosuchuserabcxyz]=$cwd/~nosuchuserabcxyz"
puts "norm-dotdot=[file normalize foo/../bar]=$cwd/bar"
puts "norm-abs=[file normalize /a/../b/./c/..]=/b"

file delete $f
puts done
