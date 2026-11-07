# `regexp -about` re_info 标志口径（oracle: tclsh 8.6.17）。
# 用法同 dialect-probes.tcl：两引擎输出 diff 应为空。
proc ta {pat} {
    if {[catch {regexp -about -- $pat} m]} {
        puts "A<$pat> => ERR <$m>"
    } else {
        puts "A<$pat> => <$m>"
    }
}
foreach p [list "" "abc" "a*" "a+" "a?" "a{0}" "a{0,2}" "a{1,2}" \
               "a{|}" "()" "(?:)" "(?=a)" "(a*)\\1" "(a)\\1" \
               "a|" "|a" "a||b" "a{,3}" "a{}b" "a{b}" "a{" "a}" \
               "(?b)" "(?b)a{2}" "(?b)a\\{2\\}" "(?b)()" "(?e)()" \
               "(?e)(a))" "(?e)a|" "\\d" "\[\[:alpha:\]\]" "\\y" \
               "a\\yb" "a\\y b" "a\\Y b" "a\\mb" "a\\Mc" "a\\M c" \
               "\\y" "\$a" "a\$b" "a\$" "^a" "a^" "a\\A" "\\Za" "a\\Z" \
               "a*?" "a+?" "(a+)(a*?)" "(a+?)(a*)" "a{1,2}?" \
               "***=" "***=(a)" "***:" "***:(a)"] {
    ta $p
}

# 强制空串 / 锚点邻接 / 词约束的 UIMPOSSIBLE 边界。
foreach p [list {^$a*\y} {^$a*\Y} {^$\y} {^$\Y} {\A\y\Z} {\A\Y\Z} \
               {^a*\y$} {^a*\m$} {^a*\M$} {a*\y^$} {\y^$} {^\M} \
               {a{b}\Z\y} {a\Z\y} {a\Z\Y} {a$\y} {a$\Y} \
               {^$(\y)} {^$(\y|a)} {^$(\y|\Y)} {^$a*\y|b}] {
    ta $p
}

# -expanded 下的 -about：注释/空白内容不得贡献 bounds/braces/locale 等
# 标志；实际忽略过空白或注释则置 REG_UNONPOSIX。
proc te {hex} {
    set pat [binary decode hex $hex]
    if {[catch {regexp -about -expanded -- $pat} m]} {
        puts "E<$hex> => ERR <$m>"
    } else {
        puts "E<$hex> => <$m>"
    }
}
te 6162
te 612062
te 61202062
te 610962
te 610a62
te 6123630a62
te 612023630a2062
te 23630a6162
te 615c2062
te 615b205d62
te 612023205c645c6e2062
te 61202320283f3d78295c6e2062
te 612023207b322c337d0a2062
te 612023205b5b3a616c7068613a5d5d0a2062
