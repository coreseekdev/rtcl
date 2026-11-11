set log {}
entry .e
pack .e
update
proc selhandler {offset maxchars} {return SELECTED}
selection own .e
puts "OWN|[selection own]"
selection handle .e selhandler
puts "SELGET|[catch {selection get} m] $m"
puts "SELGET2|[selection get]"
selection clear .e
puts "CLR|[selection own]|[catch {selection get} m] $m"
event add <<Copy>> <Control-Key-c>
event add <<Copy>> <Control-Key-Insert>
puts "VADD|[lsort [event info <<Copy>>]]"
puts "VALL|[lsort [event info]]"
bind .e <<Copy>> {lappend log copy-c}
bind .e <Control-Key-Insert> {lappend log insert}
set log {}
event generate .e <<Copy>>
puts "VGEN|$log|[lsort [event info <<Copy>>]]"
event delete <<Copy>> <Control-Key-c>
puts "VDEL|[lsort [event info <<Copy>>]]"
puts "VGEN3|[catch {event generate .e <<NoSuch>>} m] $m"
exit 0
