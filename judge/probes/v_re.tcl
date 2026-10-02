set pats [list {a**} {a**b} {a{2,1}} {[[:digit:][:foo:]]} {[[:alpha:]]} "a\\" {(ab} {a)} {[ab} {(?i)foo} {a+?} {a{1,2}?} {\Qa**\E} {(?:a|b)+c} {^(.)\1}]
foreach p $pats {
  set rc [catch {regexp $p hello} m]
  puts "$p => rc=$rc m=$m ec=$errorCode"
}
set rc [catch {regsub {a**} xxx y} m]
puts "regsub: rc=$rc m=$m ec=$errorCode"
