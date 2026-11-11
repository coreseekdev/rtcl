# Geometry measurements: canvas text bounds, text dlineinfo and the
# requested sizes of character-width widgets, compared between real Tk
# (wish) and rtcl by specs/tk/run.sh's second pass.
# Every widget is packed: real Tk answers dlineinfo only for visible lines.
set fonts {
    F1 {Helvetica 10}
    F2 {{DejaVu Sans} 12}
    F3 {Courier 10}
}
font create F1 -family Helvetica -size 10
font create F2 -family {DejaVu Sans} -size 12
font create F3 -family Courier -size 10

foreach {name spec} $fonts {
    canvas .cv -width 400 -height 200
    set id [.cv create text 50 40 -text "hello world" -font $spec -anchor nw]
    update
    puts "CB|$name|[.cv bbox $id]"
    .cv delete $id
    destroy .cv

    text .t -font $spec -width 20 -height 5
    pack .t
    .t insert end "hello\nworld wide line\nx"
    update
    puts "DL|$name L1|[.t dlineinfo 1.2]"
    puts "DL|$name L2|[.t dlineinfo 2.5]"
    puts "DL|$name L3|[.t dlineinfo 3.5]"
    puts "DL|$name L4|[.t dlineinfo 4.0]"
    destroy .t

    entry .e -width 20 -font $spec
    pack .e
    update
    puts "EW|$name|[winfo reqwidth .e]"
    destroy .e

    listbox .lb -width 15 -height 3 -font $spec
    pack .lb
    .lb insert end a b c
    update
    puts "LW|$name|[winfo reqwidth .lb]"
    destroy .lb
}
exit 0
