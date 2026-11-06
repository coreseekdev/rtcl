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
