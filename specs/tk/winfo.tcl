toplevel .top
frame .top.f
label .top.f.l -text hi
pack .top.f.l
pack .top.f
entry .e -width 20
pack .e
update
focus .e
puts "CLS|[winfo class .e]|[winfo class .top.f]|[winfo class .]"
puts "KIDS|[lsort [winfo children .]]|[winfo children .top]|[winfo children .top.f]"
puts "PAR|[winfo parent .top.f]|[winfo parent .]"
puts "MGR|[winfo manager .e]|[winfo manager .top.f]|[winfo manager .]"
puts "MAP|[winfo ismapped .e]|[winfo ismapped .top]|[winfo viewable .e]"
puts "EXI|[winfo exists .e]|[winfo exists .nosuch]"
puts "WID|[winfo width .e]|[winfo height .e]|[winfo reqwidth .e]|[winfo reqheight .e]"
puts "TOP|[winfo toplevel .top.f.l]|[winfo toplevel .]"
puts "GEOM|[winfo geometry .e]"
puts "SCR|[winfo screenwidth .]|[winfo screenheight .]|[winfo screendepth .]"
puts "CELL|[winfo cells .]"
puts "VIS|[winfo visualsavailable .]"
puts "FN|[llength [winfo interps]] >= 0"
puts "SIB|[catch {winfo parent .nosuch} m] $m"
puts "BAD|[catch {winfo nosuch .} m] $m"
exit 0
