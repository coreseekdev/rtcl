set r [lsort {d e c b a \{ d35 d300}]
set w "a b c d d300 d35 e \\{"
puts "varvar=[expr {$r eq $w}]"
puts "lit=[expr {$r eq {a b c d d300 d35 e \{}}]"
set lit {a b c d d300 d35 e \{}
puts "litlen=[string length $lit] rlen=[string length $r] litchar=[string index $lit 19][string index $lit 20]"
