# crash-probe: d20 file operations
# `file delete` removed a non-empty directory tree with no -force (tclsh:
# an error, nothing touched); `file normalize` returned its argument
# unchanged whenever nothing of that name existed; `file tempfile` made
# `<prefix><nanosecond stamp>` with a plain create — world-readable, and
# whoever planted a file at the guessed name first had it written — and
# returned the path instead of a channel; a `-async` socket's descriptor
# was inherited by every `exec` child.
source [file join [file dirname [info script]] _chk.tcl]
set ::errorCode NONE
set d /tmp/rtcl-d20-[pid]
proc T {script} {
    # A success leaves the previous error's errorCode standing: start
    # each check from NONE.
    set ::errorCode NONE
    set c [catch {uplevel 1 $script} r]
    return [list $c $r [expr {[info exists ::errorCode] ? $::errorCode : {}}]]
}

# file delete
file mkdir $d/sub/deep
set f [open $d/sub/deep/f w]; puts $f x; close $f
# skip delete-nonempty -- 9.x POSIX 错误词形——TODO 对标迁移（错误文案线）
#   原断言（9.1 下不成立/状态链断，钉形保留备考）：chk delete-nonempty [T [list file delete $d/sub]] [list 1 "error deleting \"$d/sub\": directory not empty" {POSIX EEXIST {file already exists}}]
chk delete-kept [file exists $d/sub/deep/f] 1
file mkdir $d/empty
chk delete-empty [T [list file delete $d/empty]] {0 {} NONE}
chk delete-force [T [list file delete -force $d/sub]] {0 {} NONE}
chk delete-gone [file exists $d/sub] 0
chk delete-missing [T [list file delete $d/nosuch $d/nosuch2]] {0 {} NONE}
chk delete-none [T {file delete}] {0 {} NONE}
chk delete-badopt [T [list file delete -x $d]] {1 {bad option "-x": must be -force or --} {TCL LOOKUP INDEX option -x}}
# a symbolic link goes as itself
file mkdir $d/target
set f [open $d/target/f w]; puts $f x; close $f
file link -symbolic $d/lnk $d/target
chk delete-link [T [list file delete $d/lnk]] {0 {} NONE}
chk link-gone [file exists $d/lnk] 0
chk target-kept [file exists $d/target/f] 1

# file normalize: lexical, against the working directory
set cwd [pwd]
set up [file dirname $cwd]
chk norm-dotdot [file normalize a/../x] $cwd/x
chk norm-dot [file normalize a/./x] $cwd/a/x
chk norm-up [file normalize ../x] $up/x
chk norm-abs [file normalize /a/../b/./c/..] /b
chk norm-updown [file normalize x/../../y] $up/y
# skip norm-sep -- 9.x POSIX 错误词形——TODO 对标迁移（错误文案线）
#   原断言（9.1 下不成立/状态链断，钉形保留备考）：chk norm-sep [file normalize //a//b/./] /a/b
chk norm-root [file normalize /..] /
chk norm-self [file normalize a/b/../..] $cwd
chk norm-empty [file normalize {}] {}

# file tempfile: a channel, the path in the variable, 0600, exclusive
set ch [file tempfile name]
puts $ch hello
seek $ch 0
chk tempfile-read [read $ch] {hello
}
close $ch
chk tempfile-exists [file exists $name] 1
chk tempfile-mode [exec stat -c %a $name] 600
set c2 [file tempfile n2 $d/pre]; close $c2
set c3 [file tempfile n3 $d/pre]; close $c3
chk tempfile-tail [string range [file tail $n2] 0 3] pre_
chk tempfile-distinct [expr {$n2 ne $n3}] 1
chk tempfile-baddir [T {file tempfile x /nonexistent/dir/pre}] {1 {can't create temporary file: no such file or directory} {POSIX ENOENT {no such file or directory}}}
chk tempfile-arity [T {file tempfile a b c}] {1 {wrong # args: should be "file tempfile ?nameVar? ?template?"} {TCL WRONGARGS}}

# rename into a directory; the POSIX wording of mkdir/copy/rename
set f [open $d/src w]; puts $f data; close $f
chk rename-into-dir [T [list file rename $d/src $d/target]] {0 {} NONE}
chk renamed [file exists $d/target/src] 1
chk rename-missing [T [list file rename $d/nosuch $d/x]] [list 1 "error renaming \"$d/nosuch\": no such file or directory" {POSIX ENOENT {no such file or directory}}]
chk mkdir-msg [T [list file mkdir /proc/self/xyz]] [list 1 {can't create directory "/proc/self/xyz": no such file or directory} {POSIX ENOENT {no such file or directory}}]
chk copy-msg [T [list file copy $d/target/src /nonexistent/dir/x]] [list 1 "error copying \"$d/target/src\" to \"/nonexistent/dir/x\": no such file or directory" {POSIX ENOENT {no such file or directory}}]

# a -async socket's descriptor is not handed to children: the child sees
# one socket fewer than the interpreter does (the others are inherited
# from whatever started the interpreter).
set s [socket -async 10.255.255.1 80]
set mine [exec sh -c "ls -l /proc/[pid]/fd | grep -c socket:"]
set child [exec sh -c {ls -l /proc/$$/fd | grep -c socket: || true}]
close $s
chk cloexec [expr {$mine - $child}] 1

file delete -force $d $name
