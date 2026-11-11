foreach w {entry text listbox canvas scale scrollbar message spinbox labelframe notebook menubutton} {
    catch {destroy .$w}
}
entry .entry
text .text
listbox .listbox
canvas .canvas
scale .scale
scrollbar .scrollbar
message .message
spinbox .spinbox
labelframe .labelframe
ttk::notebook .notebook
ttk::combobox .combobox
ttk::progressbar .progressbar
menubutton .menubutton
foreach w {entry text listbox canvas scale scrollbar message spinbox labelframe notebook combobox progressbar menubutton} {
    catch {$w nosuch} m
    puts "S\t$w\t[lindex [split $m {:}] 1]"
}
catch {menu .menu; .menu nosuch} m
puts "S\tmenu\t[lindex [split $m {:}] 1]"
exit 0
