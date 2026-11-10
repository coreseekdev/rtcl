# Font measurements: the same script runs under real Tk (wish) and under
# rtcl's GTK-backed runtime (tkprobe), and specs/tk/run.sh compares the two.
# Families are ones fontconfig resolves for both, so the numbers are of the
# same faces; named fonts (TkDefaultFont) are each toolkit's own choice and
# are left out.
set fonts {
    {Helvetica 10} {Helvetica 14} {Helvetica 24}
    {Courier 10} {Times 12}
    {{DejaVu Sans} 10} {{DejaVu Sans Mono} 10} {{DejaVu Sans} 10 bold}
    {Helvetica -16}
}
set texts [list "" a ab abcd "Hello, world" iiiii WWWWW 0 0000000000 \
    "The quick brown fox"]
puts "S\tscaling\t\t[format %.4f [tk scaling]]"
foreach font $fonts {
    foreach text $texts {
        puts "M\t$font\t$text\t[font measure $font $text]"
    }
    foreach option {-ascent -descent -linespace -fixed} {
        puts "X\t$font\t$option\t[font metrics $font $option]"
    }
}
exit 0
