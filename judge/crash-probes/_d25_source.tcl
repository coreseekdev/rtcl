# Source of judge/crash-probes/d25_string_index.tcl: `string index`,
# `range`, `first`, `last` — every form of index, at the ends and past
# them.  `_gen.tcl` runs every line under tclsh and writes the probe with
# tclsh's outputs as the expected values.
#
# These went by way of the whole text collected into characters first
# (every `string index` on a 24KB string cost 8µs), and `first`/`last`
# compared a slice at every position.  The answers must be the same by the
# faster roads.
set s "abcé日xyz"
set big [string repeat x 1030]

T {idx-0} {string index $::s 0}
T {idx-1} {string index $::s 1}
T {idx-3} {string index $::s 3}
T {idx-4} {string index $::s 4}
T {idx-5} {string index $::s 5}
T {idx-6} {string index $::s 6}
T {idx-8} {string index $::s 8}
T {idx-9} {string index $::s 9}
T {idx-end} {string index $::s end}
T {idx-end-1} {string index $::s end-1}
T {idx-end-9} {string index $::s end-9}
T {idx-end-10} {string index $::s end-10}
T {idx-end-1000} {string index $::s end-1000}
T {idx-end+0} {string index $::s end+0}
T {idx-end+1} {string index $::s end+1}
T {idx--1} {string index $::s -1}
T {idx--9} {string index $::s -9}
T {idx--10} {string index $::s -10}
T {idx- 1} {string index $::s " 1"}
T {idx-+1} {string index $::s +1}
T {idx- 2 } {string index $::s " 2 "}
T {idx-0x2} {string index $::s 0x2}
T {idx-0x10} {string index $::s 0x10}
T {idx-2 +1} {string index $::s "2 +1"}
T {idx-2 -1} {string index $::s "2 -1"}
T {idx-end - 2} {string index $::s "end - 2"}
T {idx-end- 2} {string index $::s "end- 2"}
T {idx-end--1} {string index $::s end--1}
T {idx-end+-1} {string index $::s end+-1}
T {idx-4294967295} {string index $::s 4294967295}
T {idx-2147483648} {string index $::s 2147483648}
T {idx--2147483649} {string index $::s -2147483649}
T {idx-99999999999999999999} {string index $::s 99999999999999999999}
T {idx-1.0} {string index $::s 1.0}
T {idx-abc} {string index $::s abc}
T {idx-2a} {string index $::s 2a}
T {idx-} {string index $::s ""}
T {idx-end5} {string index $::s end5}
T {idx-5end} {string index $::s 5end}
T {idx-2 end} {string index $::s "2 end"}
T {idx-emptyst} {string index {} 0}
T {idx-emptend} {string index {} end}
T {idx-big-1020} {string index $::big 1020}
T {idx-big-end} {string index $::big end-3}
T {idx-big-past} {string index $::big 5000}
T {idx-big-past2} {string index $::big 4294967294}
T {idx-big-neg} {string index $::big -1}
T {rng-0-2} {string range $::s 0 2}
T {rng-2-0} {string range $::s 2 0}
T {rng-0-end} {string range $::s 0 end}
T {rng-1-2} {string range $::s 1 2}
T {rng-2-1} {string range $::s 2 1}
T {rng-1-end} {string range $::s 1 end}
T {rng-4-2} {string range $::s 4 2}
T {rng-2-4} {string range $::s 2 4}
T {rng-4-end} {string range $::s 4 end}
T {rng-5-2} {string range $::s 5 2}
T {rng-2-5} {string range $::s 2 5}
T {rng-5-end} {string range $::s 5 end}
T {rng--1-2} {string range $::s -1 2}
T {rng-2--1} {string range $::s 2 -1}
T {rng--1-end} {string range $::s -1 end}
T {rng--5-2} {string range $::s -5 2}
T {rng-2--5} {string range $::s 2 -5}
T {rng--5-end} {string range $::s -5 end}
T {rng--6-2} {string range $::s -6 2}
T {rng-2--6} {string range $::s 2 -6}
T {rng--6-end} {string range $::s -6 end}
T {rng-end-2} {string range $::s end 2}
T {rng-2-end} {string range $::s 2 end}
T {rng-end-end} {string range $::s end end}
T {rng-end-1-2} {string range $::s end-1 2}
T {rng-2-end-1} {string range $::s 2 end-1}
T {rng-end-1-end} {string range $::s end-1 end}
T {rng-end-8-2} {string range $::s end-8 2}
T {rng-2-end-8} {string range $::s 2 end-8}
T {rng-end-8-end} {string range $::s end-8 end}
T {rng-end-9-2} {string range $::s end-9 2}
T {rng-2-end-9} {string range $::s 2 end-9}
T {rng-end-9-end} {string range $::s end-9 end}
T {rng-end+1-2} {string range $::s end+1 2}
T {rng-2-end+1} {string range $::s 2 end+1}
T {rng-end+1-end} {string range $::s end+1 end}
T {rng- 1-2} {string range $::s " 1" 2}
T {rng-2- 1} {string range $::s 2 " 1"}
T {rng- 1-end} {string range $::s " 1" end}
T {rng-+1-2} {string range $::s +1 2}
T {rng-2-+1} {string range $::s 2 +1}
T {rng-+1-end} {string range $::s +1 end}
T {rng-0x2-2} {string range $::s 0x2 2}
T {rng-2-0x2} {string range $::s 2 0x2}
T {rng-0x2-end} {string range $::s 0x2 end}
T {rng-1 +1-2} {string range $::s "1 +1" 2}
T {rng-2-1 +1} {string range $::s 2 "1 +1"}
T {rng-1 +1-end} {string range $::s "1 +1" end}
T {rng-1 -1-2} {string range $::s "1 -1" 2}
T {rng-2-1 -1} {string range $::s 2 "1 -1"}
T {rng-1 -1-end} {string range $::s "1 -1" end}
T {rng-end--1-2} {string range $::s end--1 2}
T {rng-2-end--1} {string range $::s 2 end--1}
T {rng-end--1-end} {string range $::s end--1 end}
T {rng-4294967295-2} {string range $::s 4294967295 2}
T {rng-2-4294967295} {string range $::s 2 4294967295}
T {rng-4294967295-end} {string range $::s 4294967295 end}
T {rng-2147483648-2} {string range $::s 2147483648 2}
T {rng-2-2147483648} {string range $::s 2 2147483648}
T {rng-2147483648-end} {string range $::s 2147483648 end}
T {rng--2147483649-2} {string range $::s -2147483649 2}
T {rng-2--2147483649} {string range $::s 2 -2147483649}
T {rng--2147483649-end} {string range $::s -2147483649 end}
T {rng-9999999999-2} {string range $::s 9999999999 2}
T {rng-2-9999999999} {string range $::s 2 9999999999}
T {rng-9999999999-end} {string range $::s 9999999999 end}
T {rng-1.0-2} {string range $::s 1.0 2}
T {rng-2-1.0} {string range $::s 2 1.0}
T {rng-1.0-end} {string range $::s 1.0 end}
T {rng-abc-2} {string range $::s abc 2}
T {rng-2-abc} {string range $::s 2 abc}
T {rng-abc-end} {string range $::s abc end}
T {rng--2} {string range $::s "" 2}
T {rng-2-} {string range $::s 2 ""}
T {rng--end} {string range $::s "" end}
T {rng-empty} {string range {} 0 end}
T {rng-empty2} {string range {} -5 5}
T {rng-rev} {string range $::s 5 2}
T {rng-big} {string range $::big 5 1020}
T {rng-bigneg} {string range $::big -5000 5000}
T {rng-big-all} {string length [string range $::big 0 end]}
T {len-ascii} {string length $::big}
T {len-uni} {string length $::s}
T {len-empty} {string length {}}
T {bytelen} {string bytelength $::s}
set h "abcdabcdabcd"
T {first-a-0} {string first a $::h 0}
T {last-a-0} {string last a $::h 0}
T {first-ab-0} {string first ab $::h 0}
T {last-ab-0} {string last ab $::h 0}
T {first-ab-2} {string first ab $::h 2}
T {last-ab-2} {string last ab $::h 2}
T {first-ab-9} {string first ab $::h 9}
T {last-ab-9} {string last ab $::h 9}
T {first-ab-10} {string first ab $::h 10}
T {last-ab-10} {string last ab $::h 10}
T {first-ab-11} {string first ab $::h 11}
T {last-ab-11} {string last ab $::h 11}
T {first-ab-12} {string first ab $::h 12}
T {last-ab-12} {string last ab $::h 12}
T {first-ab-13} {string first ab $::h 13}
T {last-ab-13} {string last ab $::h 13}
T {first-ab--1} {string first ab $::h -1}
T {last-ab--1} {string last ab $::h -1}
T {first-ab-end} {string first ab $::h end}
T {last-ab-end} {string last ab $::h end}
T {first-ab-end-3} {string first ab $::h end-3}
T {last-ab-end-3} {string last ab $::h end-3}
T {first-ab-end-4} {string first ab $::h end-4}
T {last-ab-end-4} {string last ab $::h end-4}
T {first-z-0} {string first z $::h 0}
T {last-z-0} {string last z $::h 0}
T {first--0} {string first "" $::h 0}
T {last--0} {string last "" $::h 0}
T {first-ab-4294967295} {string first ab $::h 4294967295}
T {last-ab-4294967295} {string last ab $::h 4294967295}
T {first-abcdabcdabcd-0} {string first abcdabcdabcd $::h 0}
T {last-abcdabcdabcd-0} {string last abcdabcdabcd $::h 0}
T {first-abcdabcdabcd-1} {string first abcdabcdabcd $::h 1}
T {last-abcdabcdabcd-1} {string last abcdabcdabcd $::h 1}
T {first-abcdabcdabcd-end-11} {string first abcdabcdabcd $::h end-11}
T {last-abcdabcdabcd-end-11} {string last abcdabcdabcd $::h end-11}
T {first-é-0} {string first é $::h 0}
T {last-é-0} {string last é $::h 0}
T {first-é日-0} {string first é日 $::h 0}
T {last-é日-0} {string last é日 $::h 0}
T {first-nostart} {string first cd $::h}
T {last-nostart} {string last cd $::h}
T {first-bad} {string first cd $::h x}
T {last-bad} {string last cd $::h 1.5}
set u "日本語日本abc日本"
T {ufirst-日本-0} {string first 日本 $::u 0}
T {ulast-日本-0} {string last 日本 $::u 0}
T {ufirst-本-1} {string first 本 $::u 1}
T {ulast-本-1} {string last 本 $::u 1}
T {ufirst-本-2} {string first 本 $::u 2}
T {ulast-本-2} {string last 本 $::u 2}
T {ufirst-abc-0} {string first abc $::u 0}
T {ulast-abc-0} {string last abc $::u 0}
T {ufirst-abc-3} {string first abc $::u 3}
T {ulast-abc-3} {string last abc $::u 3}
T {ufirst-abc-4} {string first abc $::u 4}
T {ulast-abc-4} {string last abc $::u 4}
T {ufirst-d-0} {string first d $::u 0}
T {ulast-d-0} {string last d $::u 0}
T {ufirst-日本-end} {string first 日本 $::u end}
T {ulast-日本-end} {string last 日本 $::u end}
T {ufirst-日本-end-3} {string first 日本 $::u end-3}
T {ulast-日本-end-3} {string last 日本 $::u end-3}
T {ufirst-本--1} {string first 本 $::u -1}
T {ulast-本--1} {string last 本 $::u -1}
T {ufirst-日本abc-2} {string first 日本abc $::u 2}
T {ulast-日本abc-2} {string last 日本abc $::u 2}
T {ufirst-語日本-end-2} {string first 語日本 $::u end-2}
T {ulast-語日本-end-2} {string last 語日本 $::u end-2}
T {ufirst-long} {string first [string repeat x 700] [string repeat xy 500] 0}
T {ulast-long} {string last [string repeat é 700] [string repeat éü 500]}
T {ufirst-boundary} {string first éé "xééé" 1}
T {ulast-boundary} {string last éé "éééx" end}
T {first-same} {string first abcdabcdabcd $::h}
T {first-bigger} {string first abcdabcdabcdx $::h}
T {last-longer} {string last abcdabcdabcdx $::h}
T {first-empty-needle} {string first {} abc}
T {last-empty-needle} {string last {} abc}
puts DONE
