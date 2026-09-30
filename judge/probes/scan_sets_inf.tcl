proc sh {s} { puts $s }
set r [scan {a} {%[^b]%c} x y]; sh "hat: r=$r x=[info exists x] y=[info exists y]"
set r [scan {aaa} {%2[^b]%c} x y]; sh "hat2: r=$r x=$x y=$y"
set r [scan {ab} {%[%]} x]; sh "pctset: r=$r x=[info exists x]"
foreach str {NaN na inf in i nan(1) -nan infinity +inf INF iNx -Inf} {
    unset -nocomplain v
    set r [scan $str %f v]
    sh "f|$str: r=$r set=[info exists v] v=[expr {[info exists v] ? [list $v] : {''}}]"
}
set r [scan 123 {%d%n%n} a b c]; sh "nn: r=$r a=$a b=$b c=$c"
set r [scan {12 34} {%d %2d%d} a b c]; sh "w2: r=$r a=$a b=$b c=$c"
set r [scan abc {%*s%d} a]; sh "supp: r=$r a=[info exists a]"
set r [scan abc {%*s} a]; sh "supp2: r=$r a=[info exists a]"
set r [scan {x  42} {%s %*d %d} p q]; sh "supp3: r=$r p=$p q=[info exists q]"
