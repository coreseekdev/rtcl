set ch {
  \u1040
  \uABF0
  \u021F
  \u0220
  \u037F
  \u052F
  \uFBC1
  \u0120
  \u00AD
  \u0605
  \u061C
  \u180E
  \u2066
  \uFEFF
  \u1680
  \u202F
  \u200B
  \u2060
  \u203F
  \u2040
  \u2054
  \uFE33
  \uFE4D
  \uFF3F
  \u0085
  \u2028
  \u2029
  \u00A0
  \u0009
  \u0020
  \u0030
  \u0041
  \u0061
  \u005F
  \u00B7
  \u002E
  \u002D
  \u0300
  \u0483
  \u0591
  \u0660
  \u06F0
  \u0966
  \u09E6
  \u0A66
  \u0BE6
  \u0C66
  \u0D66
  \u0E50
  \u0ED0
  \u0F20
  \u1090
  \u17E0
  \u1810
  \u1946
  \u19D0
  \u1A80
  \u1A90
  \u1B50
  \u1BB0
  \u1C40
  \u1C50
  \uA620
  \uA8D0
  \uA900
  \uA9D0
  \uAA50
  \uFF10
  \u104A0
  \u1D7CE
  \u11066
  \u9FB0
  \u10D30
  \u11DA0
  \u16AC0
  \u1E140
  \u1E950
  \u03FF
  \u0531
  \u0561
  \u10A0
  \u2D00
  \u1C90
  \u10D0
  \u1E9E
  \u17F4
  \u0188
}
foreach cls {alnum alpha digit space control print graph punct lower upper xdigit} {
  set out {}
  foreach c $ch { lappend out [string is $cls -strict $c] }
  puts "is:$cls $out"
}
foreach cls {alnum alpha digit space cntrl print graph punct lower upper xdigit blank word} {
  set out {}
  foreach c $ch { lappend out [regexp "\[\[:$cls:\]\]" $c] }
  puts "re:$cls $out"
}
  set out {}; foreach c $ch { lappend out [regexp "^\d$" $c] }; puts "esc:d $out"
  set out {}; foreach c $ch { lappend out [regexp "^\w$" $c] }; puts "esc:w $out"
  set out {}; foreach c $ch { lappend out [regexp "^\s$" $c] }; puts "esc:s $out"
