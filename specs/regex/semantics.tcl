# 正则语义差分语料：Tcl Spencer 引擎 vs rtcl（fast=regex crate / BT 回退）
# 输出每条的 -inline 结果（含捕获），用于统计语义差异类别。
proc t {pat str} {
    set rc [catch {regexp -inline -- $pat $str} m]
    puts "PAT<$pat> STR<$str> => $rc <$m>"
}
set pats {
    {a|ab} {ab|a} {(a|ab)} {(a|ab)c} {(ab|a)c} {(a|ab)(c|bcd)}
    {a*} {a+} {a?} {a{2,3}} {a{2,3}?} {a*?} {a+?} {a??}
    {(a+)(a*)} {(a*)(a+)} {(a+?)(a*)} {(a*?)(a+)} {(a+)(a*?)} {(a*)(a+?)}
    {(a+?)(a+?)} {(a*?)(a*?)} {(a+?)(a+)} {(a*?)(a*)} {(a??)(a*)}
    {(a*?)(a*)(a*)} {(a+?)(a*)(a+?)} {(a+)(a*?)b} {(a+?)(a*)b}
    {(a*)(a*)$} {(a*?)(a*)$} {(a|ab)$} {(ab|a)$}
    {(a|b)*} {(a|b)+?} {(ab|a)+} {(a|ab)+}
    {(a?){3}} {(a??){3}} {(a*){2,3}?}
    {[a-z]+} {[[:alpha:]]+} {[^a]+} {[a-c]+?}
    {a.*b} {a.*?b} {a.+b} {a.+?b}
    {^a+$} {^a+?$} {\Aa+} {a+?\z}
    {(a)(b)?} {(a)?(b)} {(a)?(b)?} {(a?)(b?)}
    {(a|b)(a|b)} {(ab?)(a?b)} {(a*b*)} {(a*?b*?)}
    {\d+} {\w+} {\s+} {\y} {\m} {\M}
    {(a+)\1} {(a*)\1} {(a|ab)\1}
}
set strs {"" a aa aaa ab abc abab aaab aabb abcd x}
foreach p $pats {
    foreach s $strs {
        t $p $s
    }
}
