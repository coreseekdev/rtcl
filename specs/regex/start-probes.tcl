# `regexp -start` 索引语法口径（oracle: tclsh 8.6.17）。
# C 风格整数（0x/0o/0b/前导 0 八进制）、外层空白、int±int（尾项可带
# 符号）、end±int、32 位回绕与越界错误。
proc p {st} {
    if {[catch {regexp -start $st -indices -inline {$} "abc"} m]} {
        puts "S<$st> ERR"
    } else {
        puts "S<$st> => <$m>"
    }
}
foreach st [list 0 1 2 -1 -100 end end+1 end-1 end-100 end+100 1+2 1-2 +1 1+ \
                 0x10 0X1F 010 0o10 0b10 0d10 +0x10 -0x10 " 0x10 " abc " 1" "1 " \
                 "" --1 2147483647 2147483648 4294967295 4294967296 \
                 -2147483649 -4294967295 -4294967296 -3000000000 \
                 99999999999999999999 end*2 1.5 3 \
                 "1 + 2" "1 +2" "1+ 2" " 1+2" "1+2 " "end + 1" \
                 "end+-1" "1+-2" "1--2" "  "] {
    p $st
}
