ttk::treeview .tv -columns {a b c} -show {tree headings}
foreach col {a b c} {
    .tv heading $col -text [string toupper $col]
    .tv column $col -width 100
}
pack .tv -fill both
.tv insert {} end -id r1 -text R1 -values {v1 v2}
update
puts "DRAGCOL|[catch {.tv drag a end} m] $m|[.tv column a -width]|[.tv column c -width]"
puts "DRAGBAD|[catch {.tv drag nosuch end} m] $m"
puts "DRAGSELF|[catch {.tv drag a a end} m] $m"
puts "DROP|[catch {.tv drop nosuch} m] $m"
puts "DROPHELP|[catch {.tv drop} m] $m"
puts "DRAGHELP|[catch {.tv drag} m] $m"
exit 0
