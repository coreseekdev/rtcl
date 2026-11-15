#!/usr/bin/env bash
# IO/通道组残余差分（文件面，自包含）：通道编号/复用、chan names、
# read 计数（UTF-8 多字节边界/二进制字节计数）、fblocked、chan copy
# -size/-command、eofchar 残面。双解释器同脚本比对。
set -u
cd "$(dirname "$0")/../.."
RTCL=./target/release/rtcl

CLI=$(mktemp /tmp/iores.XXXX)
O1=$(mktemp /tmp/iores_o1.XXXX)
O2=$(mktemp /tmp/iores_o2.XXXX)

cat > "$CLI" <<'EOF'
if {![info exists ::errorCode]} {set ::errorCode NONE}
proc t {label script} {
  set c [catch {uplevel 1 $script} r]
  puts "$label|$c|$r|EC=$::errorCode"
}
file delete /tmp/rtcl_io_res_a.txt
file delete /tmp/rtcl_io_res_b.txt

# ── 通道编号/复用 ──
t ids1 {list [open /tmp/rtcl_io_res_a.txt w] [open /tmp/rtcl_io_res_b.txt w]}
t ids-close-reuse {set f [open /tmp/rtcl_io_res_a.txt w]; set g [open /tmp/rtcl_io_res_b.txt w]; close $f; set h [open /tmp/rtcl_io_res_a.txt w]; list $g $h}
t names1 {chan names}
t close-all {foreach c [chan names] {if {$c ni {stdin stdout stderr}} {catch {close $c}}}; chan names}
t ids2 {list [open /tmp/rtcl_io_res_a.txt w] [open /tmp/rtcl_io_res_b.txt w+]}
t close-write-half {set f [lindex [chan names] 0]; chan close $f write; chan names}

# ── read 计数：UTF-8 多字节边界 ──
set f [open /tmp/rtcl_io_res_a.txt w]
fconfigure $f -translation lf
puts -nonewline $f "a\xe4\xb8\xadb\xc3\xa9c"
close $f
t read-1char {set f [open /tmp/rtcl_io_res_a.txt r]; set r [read $f 1]; set n [tell $f]; close $f; list $r $n}
t read-2char {set f [open /tmp/rtcl_io_res_a.txt r]; set r [read $f 2]; set n [tell $f]; close $f; list $r $n}
t read-3char {set f [open /tmp/rtcl_io_res_a.txt r]; set r [read $f 3]; set n [tell $f]; close $f; list $r $n}
t read-99char {set f [open /tmp/rtcl_io_res_a.txt r]; set r [read $f 99]; set n [tell $f]; close $f; list [string length $r] $n}
t read-binary-bytes {set f [open /tmp/rtcl_io_res_a.txt r]; fconfigure $f -translation binary; set r [read $f 3]; set n [tell $f]; close $f; binary scan $r H* hex; list $hex $n}
t read-all-then-count {set f [open /tmp/rtcl_io_res_a.txt r]; set r [read $f]; close $f; list [string length $r] [string bytelength $r]}
t gets-count-utf8 {set f [open /tmp/rtcl_io_res_a.txt r]; set line [gets $f len]; close $f; list $len [string length $line]}

# ── fblocked 元数 ──
t fblocked-file {set f [open /tmp/rtcl_io_res_a.txt r]; set r [fblocked $f]; close $f; set r}
t fblocked-arity {catch {fblocked} m; set m}
t fblocked-closed {fblocked nosuchchan}

# ── chan copy -size / -command 残余 ──
set f [open /tmp/rtcl_io_res_a.txt r]
t copy-size-half {set g [open /tmp/rtcl_io_res_b.txt w]; set n [chan copy $f $g -size 3]; close $g; set n}
t copy-tell-after {tell $f}
t copy-rest {set g [open /tmp/rtcl_io_res_b.txt w]; set n [chan copy $f $g]; close $g; close $f; set n}
t copy-b-out {set g [open /tmp/rtcl_io_res_b.txt r]; set r [read $g]; close $g; binary scan $r H* hex; set hex}
t copy-size-0 {set f [open /tmp/rtcl_io_res_a.txt r]; set g [open /tmp/rtcl_io_res_b.txt w]; set n [chan copy $f $g -size 0]; close $g; close $f; set n}
t copy-cmd-sync {set f [open /tmp/rtcl_io_res_a.txt r]; set g [open /tmp/rtcl_io_res_b.txt w]; set n [chan copy $f $g -command cb]; set ::done 1; set n}
proc cb {bytes} { incr ::copied }

# ── eofchar 残面 ──
file delete /tmp/rtcl_io_res_e.txt
t eofchar-write-close {set f [open /tmp/rtcl_io_res_e.txt w]; fconfigure $f -eofchar [list {} \x03]; puts $f hi; close $f; set g [open /tmp/rtcl_io_res_e.txt r]; fconfigure $g -translation binary; set r [read $g]; close $g; binary scan $r H* hex; set hex}
file delete /tmp/rtcl_io_res_e.txt
set f [open /tmp/rtcl_io_res_e.txt w]
fconfigure $f -translation binary
puts -nonewline $f "one\x1a two"
close $f
t eofchar-read-trunc {set f [open /tmp/rtcl_io_res_e.txt r]; fconfigure $f -eofchar [list \x1a {}]; set a [read $f]; set e [eof $f]; close $f; list $a $e}
t eofchar-gets-trunc {set f [open /tmp/rtcl_io_res_e.txt r]; fconfigure $f -eofchar [list \x1a {}]; set a [gets $f]; set e [eof $f]; close $f; list $a $e}
t eofchar-query-ro {set f [open /tmp/rtcl_io_res_e.txt r]; set r [fconfigure $f -eofchar]; close $f; set r}
t eofchar-query-wo {set f [open /tmp/rtcl_io_res_e.txt w]; set r [fconfigure $f -eofchar]; close $f; set r}
t eofchar-query-rw {set f [open /tmp/rtcl_io_res_e.txt w+]; set r [fconfigure $f -eofchar]; close $f; set r}
t eofchar-clear {set f [open /tmp/rtcl_io_res_e.txt w]; fconfigure $f -eofchar {Z Z}; set a [fconfigure $f -eofchar]; fconfigure $f -eofchar {}; set b [fconfigure $f -eofchar]; close $f; list $a $b}
exit 0
EOF

timeout 20 "$RTCL" -f "$CLI" > "$O1" 2>&1; R1=$?
timeout 20 tclsh "$CLI" > "$O2" 2>&1; R2=$?
echo "--- rtcl(rc=$R1) ---"; cat "$O1"
echo "--- tclsh(rc=$R2) ---"; cat "$O2"
echo "--- diff ---"
diff "$O2" "$O1" && echo SAME || echo DIFF
rm -f "$CLI" "$O1" "$O2"
