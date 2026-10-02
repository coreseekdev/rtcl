set p0 {a)}
catch {regexp $p0 x} m; puts "0 <{[set m]}> <{[set errorCode]}>"
set p1 {a[b}
catch {regexp $p1 x} m; puts "1 <{[set m]}> <{[set errorCode]}>"
set p2 {a{2,1}}
catch {regexp $p2 x} m; puts "2 <{[set m]}> <{[set errorCode]}>"
set p3 {*a}
catch {regexp $p3 x} m; puts "3 <{[set m]}> <{[set errorCode]}>"
set p4 {a**}
catch {regexp $p4 x} m; puts "4 <{[set m]}> <{[set errorCode]}>"
set p5 {[[:alpha:]]}
catch {regexp $p5 x} m; puts "5 <{[set m]}> <{[set errorCode]}>"
set p6 {a**b}
catch {regexp $p6 x} m; puts "6 <{[set m]}> <{[set errorCode]}>"
set p7 {[[:digit:][:foo:]]}
catch {regexp $p7 x} m; puts "7 <{[set m]}> <{[set errorCode]}>"
set p8 "a\\"
catch {regexp $p8 x} m; puts "8 <{[set m]}> <{[set errorCode]}>"
set p9 "a\d+"
catch {regexp $p9 x} m; puts "9 <{[set m]}> <{[set errorCode]}>"
