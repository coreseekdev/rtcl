# timer (tclsh 9.1 monotonic timer ensemble) — deterministic faces only
# (error texts, arities, subcommand resolution; no timing values).
catch {timer} m; puts "1:$m :: $errorCode"
catch {timer gorp} m; puts "2:$m :: $errorCode"
catch {timer in 0 ms} m; puts "3:$m"
catch {timer in 2x ms s} m; puts "4:$m :: $errorCode"
catch {timer in 2 bla s} m; puts "5:$m :: $errorCode"
catch {timer at} m; puts "6:$m"
catch {timer at 2x ms s} m; puts "7:$m :: $errorCode"
catch {timer at 2 bla s} m; puts "8:$m :: $errorCode"
catch {timer sleep junk 1} m; puts "9:$m :: $errorCode"
catch {timer sleep for 1 bla} m; puts "10:$m :: $errorCode"
catch {timer sleep} m; puts "11:$m"
catch {timer sleep for} m; puts "12:$m"
catch {timer cancel} m; puts "13:$m"
catch {timer cancel a b} m; puts "14:$m"
puts "15:[catch {timer cancel unknown}]"
puts "16:[catch {timer sleep for 1 micro}]"
catch {timer idle} m; puts "17:$m"
catch {timer idle a b} m; puts "18:$m"
catch {timer info a b} m; puts "19:$m"
catch {timer info after#ab} m; puts "20:$m :: $errorCode"
catch {timer info after#} m; puts "21:$m :: $errorCode"
# real scheduling returns an after#-shaped id
set id [timer in 10 ms {set done 1}]
puts "22:[string match {after#*} $id]"
set id2 [timer idle {set idle 1}]
puts "23:[string match {after#*} $id2]"
puts "24:[expr {[lsearch -exact [lsort [timer info]] $id] >= 0}]"
# unit prefix resolution
catch {timer in 1 micro s} m; puts "25:$m"
catch {timer in 1 mic s} m; puts "26:$m"
catch {timer in 1 s s} m; puts "27:$m"
catch {timer in 1 m s} m; puts "28:$m :: $errorCode"
