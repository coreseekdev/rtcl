# crash-probe: d25 `string index`, `range`, `first`, `last` — the index
# forms at the ends and past them
# Each of these collected the whole text into characters first (every
# `string index` on a 24KB string cost 8µs, 51 times tclsh), and `first`
# and `last` compared a slice at every position.  They go by bytes now
# where the text is ASCII, and by the library's search elsewhere.
#
# Expected values are tclsh 8.6.17's own outputs, check by check, in the
# order the state is built: _gen.tcl writes this file from
# _d25_source.tcl.

source [file join [file dirname [info script]] _chk.tcl]
proc T {script} {
    # A success leaves the previous error's errorCode standing: start
    # each check from NONE.
    set ::errorCode NONE
    set c [catch {uplevel 1 $script} r]
    return [list $c $r [expr {[info exists ::errorCode] ? $::errorCode : {}}]]
}
# A skipped check still builds the state the later ones sit on: its
# script runs, unchecked.
proc U {script} {
    catch {uplevel 1 $script}
}
set ::errorCode NONE
set s "abcé日xyz"
set big [string repeat x 1030]
chk idx-0 [T {string index $::s 0}] {0 a NONE}
chk idx-1 [T {string index $::s 1}] {0 b NONE}
chk idx-3 [T {string index $::s 3}] {0 é NONE}
chk idx-4 [T {string index $::s 4}] {0 日 NONE}
chk idx-5 [T {string index $::s 5}] {0 x NONE}
chk idx-6 [T {string index $::s 6}] {0 y NONE}
chk idx-8 [T {string index $::s 8}] {0 {} NONE}
chk idx-9 [T {string index $::s 9}] {0 {} NONE}
chk idx-end [T {string index $::s end}] {0 z NONE}
chk idx-end-1 [T {string index $::s end-1}] {0 y NONE}
chk idx-end-9 [T {string index $::s end-9}] {0 {} NONE}
chk idx-end-10 [T {string index $::s end-10}] {0 {} NONE}
chk idx-end-1000 [T {string index $::s end-1000}] {0 {} NONE}
chk idx-end+0 [T {string index $::s end+0}] {0 z NONE}
chk idx-end+1 [T {string index $::s end+1}] {0 {} NONE}
chk idx--1 [T {string index $::s -1}] {0 {} NONE}
chk idx--9 [T {string index $::s -9}] {0 {} NONE}
chk idx--10 [T {string index $::s -10}] {0 {} NONE}
chk {idx- 1} [T {string index $::s " 1"}] {0 b NONE}
chk idx-+1 [T {string index $::s +1}] {0 b NONE}
chk {idx- 2 } [T {string index $::s " 2 "}] {0 c NONE}
chk idx-0x2 [T {string index $::s 0x2}] {0 c NONE}
chk idx-0x10 [T {string index $::s 0x10}] {0 {} NONE}
chk {idx-2 +1} [T {string index $::s "2 +1"}] {1 {bad index "2 +1": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk {idx-2 -1} [T {string index $::s "2 -1"}] {1 {bad index "2 -1": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk {idx-end - 2} [T {string index $::s "end - 2"}] {1 {bad index "end - 2": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk {idx-end- 2} [T {string index $::s "end- 2"}] {1 {bad index "end- 2": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk idx-end--1 [T {string index $::s end--1}] {0 {} NONE}
chk idx-end+-1 [T {string index $::s end+-1}] {0 y NONE}
chk idx-4294967295 [T {string index $::s 4294967295}] {0 {} NONE}
chk idx-2147483648 [T {string index $::s 2147483648}] {0 {} NONE}
chk idx--2147483649 [T {string index $::s -2147483649}] {0 {} NONE}
chk idx-99999999999999999999 [T {string index $::s 99999999999999999999}] {1 {bad index "99999999999999999999": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk idx-1.0 [T {string index $::s 1.0}] {1 {bad index "1.0": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk idx-abc [T {string index $::s abc}] {1 {bad index "abc": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk idx-2a [T {string index $::s 2a}] {1 {bad index "2a": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk idx- [T {string index $::s ""}] {1 {bad index "": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk idx-end5 [T {string index $::s end5}] {1 {bad index "end5": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk idx-5end [T {string index $::s 5end}] {1 {bad index "5end": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk {idx-2 end} [T {string index $::s "2 end"}] {1 {bad index "2 end": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk idx-emptyst [T {string index {} 0}] {0 {} NONE}
chk idx-emptend [T {string index {} end}] {0 {} NONE}
chk idx-big-1020 [T {string index $::big 1020}] {0 x NONE}
chk idx-big-end [T {string index $::big end-3}] {0 x NONE}
chk idx-big-past [T {string index $::big 5000}] {0 {} NONE}
chk idx-big-past2 [T {string index $::big 4294967294}] {0 {} NONE}
chk idx-big-neg [T {string index $::big -1}] {0 {} NONE}
chk rng-0-2 [T {string range $::s 0 2}] {0 abc NONE}
chk rng-2-0 [T {string range $::s 2 0}] {0 {} NONE}
chk rng-0-end [T {string range $::s 0 end}] {0 abcé日xyz NONE}
chk rng-1-2 [T {string range $::s 1 2}] {0 bc NONE}
chk rng-2-1 [T {string range $::s 2 1}] {0 {} NONE}
chk rng-1-end [T {string range $::s 1 end}] {0 bcé日xyz NONE}
chk rng-4-2 [T {string range $::s 4 2}] {0 {} NONE}
chk rng-2-4 [T {string range $::s 2 4}] {0 cé日 NONE}
chk rng-4-end [T {string range $::s 4 end}] {0 日xyz NONE}
chk rng-5-2 [T {string range $::s 5 2}] {0 {} NONE}
chk rng-2-5 [T {string range $::s 2 5}] {0 cé日x NONE}
chk rng-5-end [T {string range $::s 5 end}] {0 xyz NONE}
chk rng--1-2 [T {string range $::s -1 2}] {0 abc NONE}
chk rng-2--1 [T {string range $::s 2 -1}] {0 {} NONE}
chk rng--1-end [T {string range $::s -1 end}] {0 abcé日xyz NONE}
chk rng--5-2 [T {string range $::s -5 2}] {0 abc NONE}
chk rng-2--5 [T {string range $::s 2 -5}] {0 {} NONE}
chk rng--5-end [T {string range $::s -5 end}] {0 abcé日xyz NONE}
chk rng--6-2 [T {string range $::s -6 2}] {0 abc NONE}
chk rng-2--6 [T {string range $::s 2 -6}] {0 {} NONE}
chk rng--6-end [T {string range $::s -6 end}] {0 abcé日xyz NONE}
chk rng-end-2 [T {string range $::s end 2}] {0 {} NONE}
chk rng-2-end [T {string range $::s 2 end}] {0 cé日xyz NONE}
chk rng-end-end [T {string range $::s end end}] {0 z NONE}
chk rng-end-1-2 [T {string range $::s end-1 2}] {0 {} NONE}
chk rng-2-end-1 [T {string range $::s 2 end-1}] {0 cé日xy NONE}
chk rng-end-1-end [T {string range $::s end-1 end}] {0 yz NONE}
chk rng-end-8-2 [T {string range $::s end-8 2}] {0 abc NONE}
chk rng-2-end-8 [T {string range $::s 2 end-8}] {0 {} NONE}
chk rng-end-8-end [T {string range $::s end-8 end}] {0 abcé日xyz NONE}
chk rng-end-9-2 [T {string range $::s end-9 2}] {0 abc NONE}
chk rng-2-end-9 [T {string range $::s 2 end-9}] {0 {} NONE}
chk rng-end-9-end [T {string range $::s end-9 end}] {0 abcé日xyz NONE}
chk rng-end+1-2 [T {string range $::s end+1 2}] {0 {} NONE}
chk rng-2-end+1 [T {string range $::s 2 end+1}] {0 cé日xyz NONE}
chk rng-end+1-end [T {string range $::s end+1 end}] {0 {} NONE}
chk {rng- 1-2} [T {string range $::s " 1" 2}] {0 bc NONE}
chk {rng-2- 1} [T {string range $::s 2 " 1"}] {0 {} NONE}
chk {rng- 1-end} [T {string range $::s " 1" end}] {0 bcé日xyz NONE}
chk rng-+1-2 [T {string range $::s +1 2}] {0 bc NONE}
chk rng-2-+1 [T {string range $::s 2 +1}] {0 {} NONE}
chk rng-+1-end [T {string range $::s +1 end}] {0 bcé日xyz NONE}
chk rng-0x2-2 [T {string range $::s 0x2 2}] {0 c NONE}
chk rng-2-0x2 [T {string range $::s 2 0x2}] {0 c NONE}
chk rng-0x2-end [T {string range $::s 0x2 end}] {0 cé日xyz NONE}
chk {rng-1 +1-2} [T {string range $::s "1 +1" 2}] {1 {bad index "1 +1": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk {rng-2-1 +1} [T {string range $::s 2 "1 +1"}] {1 {bad index "1 +1": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk {rng-1 +1-end} [T {string range $::s "1 +1" end}] {1 {bad index "1 +1": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk {rng-1 -1-2} [T {string range $::s "1 -1" 2}] {1 {bad index "1 -1": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk {rng-2-1 -1} [T {string range $::s 2 "1 -1"}] {1 {bad index "1 -1": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk {rng-1 -1-end} [T {string range $::s "1 -1" end}] {1 {bad index "1 -1": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-end--1-2 [T {string range $::s end--1 2}] {0 {} NONE}
chk rng-2-end--1 [T {string range $::s 2 end--1}] {0 cé日xyz NONE}
chk rng-end--1-end [T {string range $::s end--1 end}] {0 {} NONE}
chk rng-4294967295-2 [T {string range $::s 4294967295 2}] {0 abc NONE}
chk rng-2-4294967295 [T {string range $::s 2 4294967295}] {0 {} NONE}
chk rng-4294967295-end [T {string range $::s 4294967295 end}] {0 abcé日xyz NONE}
chk rng-2147483648-2 [T {string range $::s 2147483648 2}] {0 abc NONE}
chk rng-2-2147483648 [T {string range $::s 2 2147483648}] {0 {} NONE}
chk rng-2147483648-end [T {string range $::s 2147483648 end}] {0 abcé日xyz NONE}
chk rng--2147483649-2 [T {string range $::s -2147483649 2}] {0 {} NONE}
chk rng-2--2147483649 [T {string range $::s 2 -2147483649}] {0 cé日xyz NONE}
chk rng--2147483649-end [T {string range $::s -2147483649 end}] {0 {} NONE}
chk rng-9999999999-2 [T {string range $::s 9999999999 2}] {1 {bad index "9999999999": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-2-9999999999 [T {string range $::s 2 9999999999}] {1 {bad index "9999999999": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-9999999999-end [T {string range $::s 9999999999 end}] {1 {bad index "9999999999": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-1.0-2 [T {string range $::s 1.0 2}] {1 {bad index "1.0": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-2-1.0 [T {string range $::s 2 1.0}] {1 {bad index "1.0": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-1.0-end [T {string range $::s 1.0 end}] {1 {bad index "1.0": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-abc-2 [T {string range $::s abc 2}] {1 {bad index "abc": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-2-abc [T {string range $::s 2 abc}] {1 {bad index "abc": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-abc-end [T {string range $::s abc end}] {1 {bad index "abc": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng--2 [T {string range $::s "" 2}] {1 {bad index "": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-2- [T {string range $::s 2 ""}] {1 {bad index "": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng--end [T {string range $::s "" end}] {1 {bad index "": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk rng-empty [T {string range {} 0 end}] {0 {} NONE}
chk rng-empty2 [T {string range {} -5 5}] {0 {} NONE}
chk rng-rev [T {string range $::s 5 2}] {0 {} NONE}
chk rng-big [T {string range $::big 5 1020}] {0 xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx NONE}
chk rng-bigneg [T {string range $::big -5000 5000}] {0 xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx NONE}
chk rng-big-all [T {string length [string range $::big 0 end]}] {0 1030 NONE}
chk len-ascii [T {string length $::big}] {0 1030 NONE}
chk len-uni [T {string length $::s}] {0 8 NONE}
chk len-empty [T {string length {}}] {0 0 NONE}
chk bytelen [T {string bytelength $::s}] {0 11 NONE}
set h "abcdabcdabcd"
chk first-a-0 [T {string first a $::h 0}] {0 0 NONE}
chk last-a-0 [T {string last a $::h 0}] {0 0 NONE}
chk first-ab-0 [T {string first ab $::h 0}] {0 0 NONE}
chk last-ab-0 [T {string last ab $::h 0}] {0 -1 NONE}
chk first-ab-2 [T {string first ab $::h 2}] {0 4 NONE}
chk last-ab-2 [T {string last ab $::h 2}] {0 0 NONE}
chk first-ab-9 [T {string first ab $::h 9}] {0 -1 NONE}
chk last-ab-9 [T {string last ab $::h 9}] {0 8 NONE}
chk first-ab-10 [T {string first ab $::h 10}] {0 -1 NONE}
chk last-ab-10 [T {string last ab $::h 10}] {0 8 NONE}
chk first-ab-11 [T {string first ab $::h 11}] {0 -1 NONE}
chk last-ab-11 [T {string last ab $::h 11}] {0 8 NONE}
chk first-ab-12 [T {string first ab $::h 12}] {0 -1 NONE}
chk last-ab-12 [T {string last ab $::h 12}] {0 8 NONE}
chk first-ab-13 [T {string first ab $::h 13}] {0 -1 NONE}
chk last-ab-13 [T {string last ab $::h 13}] {0 8 NONE}
chk first-ab--1 [T {string first ab $::h -1}] {0 0 NONE}
chk last-ab--1 [T {string last ab $::h -1}] {0 -1 NONE}
chk first-ab-end [T {string first ab $::h end}] {0 -1 NONE}
chk last-ab-end [T {string last ab $::h end}] {0 8 NONE}
chk first-ab-end-3 [T {string first ab $::h end-3}] {0 8 NONE}
chk last-ab-end-3 [T {string last ab $::h end-3}] {0 4 NONE}
chk first-ab-end-4 [T {string first ab $::h end-4}] {0 8 NONE}
chk last-ab-end-4 [T {string last ab $::h end-4}] {0 4 NONE}
chk first-z-0 [T {string first z $::h 0}] {0 -1 NONE}
chk last-z-0 [T {string last z $::h 0}] {0 -1 NONE}
chk first--0 [T {string first "" $::h 0}] {0 -1 NONE}
chk last--0 [T {string last "" $::h 0}] {0 -1 NONE}
chk first-ab-4294967295 [T {string first ab $::h 4294967295}] {0 0 NONE}
chk last-ab-4294967295 [T {string last ab $::h 4294967295}] {0 -1 NONE}
chk first-abcdabcdabcd-0 [T {string first abcdabcdabcd $::h 0}] {0 0 NONE}
chk last-abcdabcdabcd-0 [T {string last abcdabcdabcd $::h 0}] {0 -1 NONE}
chk first-abcdabcdabcd-1 [T {string first abcdabcdabcd $::h 1}] {0 -1 NONE}
chk last-abcdabcdabcd-1 [T {string last abcdabcdabcd $::h 1}] {0 -1 NONE}
chk first-abcdabcdabcd-end-11 [T {string first abcdabcdabcd $::h end-11}] {0 0 NONE}
chk last-abcdabcdabcd-end-11 [T {string last abcdabcdabcd $::h end-11}] {0 -1 NONE}
chk first-é-0 [T {string first é $::h 0}] {0 -1 NONE}
chk last-é-0 [T {string last é $::h 0}] {0 -1 NONE}
chk first-é日-0 [T {string first é日 $::h 0}] {0 -1 NONE}
chk last-é日-0 [T {string last é日 $::h 0}] {0 -1 NONE}
chk first-nostart [T {string first cd $::h}] {0 2 NONE}
chk last-nostart [T {string last cd $::h}] {0 10 NONE}
chk first-bad [T {string first cd $::h x}] {1 {bad index "x": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
chk last-bad [T {string last cd $::h 1.5}] {1 {bad index "1.5": must be integer?[+-]integer? or end?[+-]integer?} {TCL VALUE INDEX}}
set u "日本語日本abc日本"
chk ufirst-日本-0 [T {string first 日本 $::u 0}] {0 0 NONE}
chk ulast-日本-0 [T {string last 日本 $::u 0}] {0 -1 NONE}
chk ufirst-本-1 [T {string first 本 $::u 1}] {0 1 NONE}
chk ulast-本-1 [T {string last 本 $::u 1}] {0 1 NONE}
chk ufirst-本-2 [T {string first 本 $::u 2}] {0 4 NONE}
chk ulast-本-2 [T {string last 本 $::u 2}] {0 1 NONE}
chk ufirst-abc-0 [T {string first abc $::u 0}] {0 5 NONE}
chk ulast-abc-0 [T {string last abc $::u 0}] {0 -1 NONE}
chk ufirst-abc-3 [T {string first abc $::u 3}] {0 5 NONE}
chk ulast-abc-3 [T {string last abc $::u 3}] {0 -1 NONE}
chk ufirst-abc-4 [T {string first abc $::u 4}] {0 5 NONE}
chk ulast-abc-4 [T {string last abc $::u 4}] {0 -1 NONE}
chk ufirst-d-0 [T {string first d $::u 0}] {0 -1 NONE}
chk ulast-d-0 [T {string last d $::u 0}] {0 -1 NONE}
chk ufirst-日本-end [T {string first 日本 $::u end}] {0 -1 NONE}
chk ulast-日本-end [T {string last 日本 $::u end}] {0 8 NONE}
chk ufirst-日本-end-3 [T {string first 日本 $::u end-3}] {0 8 NONE}
chk ulast-日本-end-3 [T {string last 日本 $::u end-3}] {0 3 NONE}
chk ufirst-本--1 [T {string first 本 $::u -1}] {0 1 NONE}
chk ulast-本--1 [T {string last 本 $::u -1}] {0 -1 NONE}
chk ufirst-日本abc-2 [T {string first 日本abc $::u 2}] {0 3 NONE}
chk ulast-日本abc-2 [T {string last 日本abc $::u 2}] {0 -1 NONE}
chk ufirst-語日本-end-2 [T {string first 語日本 $::u end-2}] {0 -1 NONE}
chk ulast-語日本-end-2 [T {string last 語日本 $::u end-2}] {0 2 NONE}
chk ufirst-long [T {string first [string repeat x 700] [string repeat xy 500] 0}] {0 -1 NONE}
chk ulast-long [T {string last [string repeat é 700] [string repeat éü 500]}] {0 -1 NONE}
chk ufirst-boundary [T {string first éé "xééé" 1}] {0 1 NONE}
chk ulast-boundary [T {string last éé "éééx" end}] {0 1 NONE}
chk first-same [T {string first abcdabcdabcd $::h}] {0 0 NONE}
chk first-bigger [T {string first abcdabcdabcdx $::h}] {0 -1 NONE}
chk last-longer [T {string last abcdabcdabcdx $::h}] {0 -1 NONE}
chk first-empty-needle [T {string first {} abc}] {0 -1 NONE}
chk last-empty-needle [T {string last {} abc}] {0 -1 NONE}

# 213 lines; the chk ones are the checks.
