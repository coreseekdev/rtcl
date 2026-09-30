set ak X
puts "bare-arrow: $ak→"
puts "braced-arrow: ${ak}→"
set 变量 hello
puts "bare-cjk: $变量"
puts "braced-cjk: ${变量}"
set bk Y
puts "fwparen: $bk（hint"
puts "guillemet: $ak»"
catch {set s "$nope→"} m; puts "err: $m"
