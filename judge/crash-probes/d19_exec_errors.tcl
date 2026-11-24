# crash-probe: d19 exec errors
# `exec` reported differently from tclsh at every turn: a child that wrote
# to stderr succeeded (stderr printed by rtcl) instead of failing the
# command with that output as the message; a failing child's message
# carried "(exit code N)" and no newline after the stdout; a child killed
# by a signal was an exit 1; a success reset ::errorCode; redirection and
# spawn failures had Rust's wording and no errorCode; `2>@1` returned all
# of stdout and then all of stderr; attached targets (`2>/dev/null`) were
# passed to the command as arguments.
source [file join [file dirname [info script]] _chk.tcl]
set ::errorCode NONE

# `script`'s completion, message and errorCode (pids normalized).
proc T {script} {
    set c [catch {uplevel 1 $script} r]
    set e [regsub {CHILD(STATUS|KILLED) \d+} $::errorCode {CHILD\1 PID}]
    return [list $c $r $e]
}

# stderr output fails the exec; the message is stdout then stderr
chk stderr-fails [T {exec sh -c {echo OUT; echo ERR >&2}}] {1 {OUT
ERR} NONE}
chk stderr-only [T {exec sh -c {echo ERR >&2}}] {1 ERR NONE}
chk stderr-no-newline [T {exec sh -c {printf OUT; printf ERR >&2}}] {1 OUTERR NONE}
chk ignorestderr [lrange [T {exec -ignorestderr sh -c {echo OUT; echo ERR >&2}}] 0 1] {0 OUT}
chk stderr-kept [T {exec -keepnewline sh -c {echo OUT; echo ERR >&2}}] {1 {OUT
ERR
} NONE}
# 2>@1: both streams on one pipe, in the order they were written
chk merged [T {exec sh -c {echo 1; echo E1 >&2; echo 2; echo E2 >&2} 2>@1}] {0 {1
E1
2
E2} NONE}

# a failing child
chk exit-code [T {exec sh -c {exit 3}}] {1 {child process exited abnormally} {CHILDSTATUS PID 3}}
chk exit-stdout [T {exec sh -c {echo A; exit 3}}] {1 {A
child process exited abnormally} {CHILDSTATUS PID 3}}
chk exit-stdout-no-newline [T {exec sh -c {printf A; exit 3}}] {1 {Achild process exited abnormally} {CHILDSTATUS PID 3}}
chk exit-with-stderr [T {exec sh -c {printf A; echo B >&2; exit 3}}] {1 AB {CHILDSTATUS PID 3}}
chk exit-ignorestderr [T {exec -ignorestderr sh -c {echo OUT; echo ERR >&2; exit 3}}] {1 {OUT
child process exited abnormally} {CHILDSTATUS PID 3}}
chk exit-merged [T {exec sh -c {echo OUT; echo ERR >&2; exit 3} 2>@1}] {1 {OUT
ERR
child process exited abnormally} {CHILDSTATUS PID 3}}
chk two-failures [T {exec sh -c {exit 1} | sh -c {exit 2}}] {1 {child process exited abnormally} {CHILDSTATUS PID 2}}
chk exit-then-signal [T {exec sh -c {exit 3} | sh -c {kill -ABRT $$}}] {1 {child process exited abnormally} {CHILDKILLED PID SIGABRT SIGABRT}}

# a child killed by a signal
chk killed-term [T {exec sh -c {kill -TERM $$}}] {1 {child killed: software termination signal} {CHILDKILLED PID SIGTERM {software termination signal}}}
chk killed-segv [T {exec sh -c {kill -SEGV $$}}] {1 {child killed: segmentation violation} {CHILDKILLED PID SIGSEGV {segmentation violation}}}
chk killed-with-stderr [T {exec sh -c {echo OUT; echo ERR >&2; kill -TERM $$}}] {1 {OUT
ERR} {CHILDKILLED PID SIGTERM {software termination signal}}}

# a success leaves ::errorCode alone
set ::errorCode SENTINEL
chk success-untouched [T {exec true}] {0 {} SENTINEL}
set ::errorCode NONE

# spawn and redirection failures: tclsh's wording and POSIX code
chk no-such-command [T {exec nosuchcmdxyz-rtcl}] {1 {couldn't execute "nosuchcmdxyz-rtcl": no such file or directory} {POSIX ENOENT {no such file or directory}}}
chk read-failure [T {exec cat < /nonexistent/dir/file}] {1 {couldn't read file "/nonexistent/dir/file": no such file or directory} {POSIX ENOENT {no such file or directory}}}
chk write-failure [T {exec sh -c {echo hi} > /nonexistent/dir/file}] {1 {couldn't write file "/nonexistent/dir/file": no such file or directory} {POSIX ENOENT {no such file or directory}}}
# opened before anything runs, in command-line order
chk before-spawn [T {exec nosuchcmdxyz-rtcl 2>/nonexistent/dir/f}] {1 {couldn't write file "/nonexistent/dir/f": no such file or directory} {POSIX ENOENT {no such file or directory}}}
chk in-order [T {exec sh -c {echo hi} 2>/nonexistent/dir/b > /nonexistent/dir/a}] {1 {couldn't write file "/nonexistent/dir/b": no such file or directory} {POSIX ENOENT {no such file or directory}}}

# attached targets
set tmp /tmp/rtcl-d19-[pid]
chk attached [lrange [T {exec sh -c {echo OUT; echo ERR >&2} 2>/dev/null}] 0 1] {0 OUT}
exec sh -c {echo OUT; echo ERR >&2} >$tmp.out 2>$tmp.err
chk attached-out [exec cat $tmp.out] OUT
chk attached-err [exec cat $tmp.err] ERR
exec sh -c {echo OUT; echo ERR >&2} >>$tmp.out 2>>$tmp.err
chk appended [exec cat $tmp.out] {OUT
OUT}
chk append-both [lrange [T {exec sh -c {echo A; echo B >&2} |& cat}] 0 1] {0 {A
B}}
file delete $tmp.out $tmp.err

# argument errors
chk trailing-pipe [T {exec cat |}] {1 {illegal use of | or |& in command} {TCL OPERATION EXEC PIPESYNTAX}}
chk leading-pipe [T {exec | cat}] {1 {illegal use of | or |& in command} {TCL OPERATION EXEC PIPESYNTAX}}
chk last-word [T {exec cat >}] {1 {can't specify ">" as last word in command} {TCL OPERATION EXEC SYNTAX}}
chk last-word-attached [T {exec cat 2>}] {1 {can't specify "2>" as last word in command} {TCL OPERATION EXEC SYNTAX}}
chk merge-not-last [T {exec cat 2>@1 | cat}] {1 {must specify "2>@1" as last word in command} {TCL OPERATION EXEC PIPESYNTAX}}
chk bad-switch [T {exec -bogus cat}] {1 {bad option "-bogus": must be -ignorestderr, -keepnewline, or --} {TCL LOOKUP INDEX option -bogus}}
chk no-abbreviation [T {exec -i cat}] {1 {bad option "-i": must be -ignorestderr, -keepnewline, or --} {TCL LOOKUP INDEX option -i}}

# the output's text: one trailing newline gone, CR LF and a lone CR are
# newlines, an unknown byte is the character of its value
chk crlf [exec sh -c {printf 'OUT\r\n'}] OUT
chk keepnewline [exec -keepnewline sh -c {printf 'OUT\r\n'}] {OUT
}
chk lone-cr [exec sh -c {printf 'A\rB\n'}] {A
B}
chk bytes [lmap ch [split [exec sh -c {printf 'A\377B\200\303\264'}] ""] {format %x [scan $ch %c]}] {41 ff 42 80 f4}

# @channel redirections: the child gets the channel's own descriptor
set c [open $tmp.chan w]
exec echo hi >@ $c
close $c
chk channel-out [exec cat $tmp.chan] hi
chk channel-bad [T {exec echo hi >@ nosuchchan-rtcl}] {1 {can not find channel named "nosuchchan-rtcl"} {TCL LOOKUP CHANNEL nosuchchan-rtcl}}
set c [open $tmp.chan r]
chk channel-direction [T [list exec echo hi >@ $c]] [list 1 "channel \"$c\" wasn't opened for writing" {TCL OPERATION EXEC BADCHAN}]
set c2 [open $tmp.chan w]
chk channel-input [T [list exec cat <@ $c2]] [list 1 "channel \"$c2\" wasn't opened for reading" {TCL OPERATION EXEC BADCHAN}]
close $c
close $c2
chk channel-last-word [T {exec echo hi >@}] {1 {can't specify ">@" as last word in command} {TCL OPERATION EXEC SYNTAX}}
chk channel-stderr [lrange [T {exec sh -c {echo O; echo E >&2} 2>@ stderr}] 0 1] {0 O}
chk channel-stdout [lrange [T {exec echo hi >@ stdout}] 0 1] {0 {}}
file delete $tmp.chan

# stdin: a file, a `<<` string, and the pipe from the previous stage
set f [open $tmp.in w]
puts -nonewline $f "l1\nl2\n"
close $f
chk stdin-file [exec cat <$tmp.in] {l1
l2}
chk stdin-string [exec cat << x\ny] {x
y}
chk stdin-pipe [exec cat <$tmp.in | tr a-z A-Z] {L1
L2}
file delete $tmp.in
