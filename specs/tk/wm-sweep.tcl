toplevel .t
wm title .t "Sweep"
frame .t.f -width 150 -height 80
pack .t.f
wm geometry .t "200x100+30+40"
wm resizable .t 0 1
wm minsize .t 100 50
wm maxsize .t 800 600
wm transient .t .
wm group .t .t
wm overrideredirect .t 0
wm focusmodel .t active
wm iconname .t "Icon"
wm protocol .t WM_DELETE_WINDOW {puts bye}
puts "TITLE|[wm title .t]"
puts "RES|[wm resizable .t]"
puts "MIN|[wm minsize .t]|MAX|[wm maxsize .t]"
puts "TRANS|[wm transient .t]|GROUP|[wm group .t]"
puts "ORD|[wm overrideredirect .t]|FOCUS|[wm focusmodel .t]"
puts "ICON|[wm iconname .t]"
puts "PROTO|[wm protocol .t WM_DELETE_WINDOW]|[wm protocol .t]"
puts "STATE|[wm state .t]"
puts "STACK|[wm stackorder .]"
puts "ATTR|[wm attributes .t -fullscreen]|[catch {wm attributes .t -bogus} m] $m"
puts "BAD|[catch {wm nosuch .t} m] $m"
set v {}
after 200 {set v 1}
tkwait variable v
puts "TKWAIT-V|done"
after 150 {destroy .t.f}
tkwait window .t.f
puts "TKWAIT-W|gone"
exit 0
