foreach {name ch} {RomanNumeral Ⅷ ModLetter ˡ Hiragana あ ZeroWidthSP ​ NBSP   FigSpace   Emdash — Currency € Superscript2 ² Kelvin K Angstrom Å} {
    puts "$ch alpha=[string is alpha $ch] upper=[string is upper $ch] lower=[string is lower $ch] space=[string is space $ch] punct=[string is punct $ch] graph=[string is graph $ch] print=[string is print $ch] alnum=[string is alnum $ch] digit=[string is digit $ch] cntrl=[string is control $ch]"
}
puts "DEL cntrl=[string is control \x7f]"
puts "zwsp space=[string is space ​] graph=[string is graph ​]"
puts "combining acute: alpha=[string is alpha ́] graph=[string is graph ́] print=[string is print ́]"
