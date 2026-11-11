set log {}
proc rec {tag args} {
    lappend ::log "$tag N=[llength $args] [join $args ,]"
}
entry .e
pack .e
update
focus .e
# Key events route through the focus chain, so they are bound on all:
# whichever window Tk's focus lands on records the substitutions.
bind all <Key-a> {rec key-a %K %k %s %A}
bind all <Key-Return> {rec key-ret %K %s %A}
bind .e <Button-1> {rec btn1 %b %s %x %y %W}
bind .e <Double-Button-1> {rec dbl1 %b %s %x %y}
bind .e <Triple-Button-1> {rec tpl1 %b %s}
bind .e <ButtonRelease-1> {rel %b %s %x %y}
proc rel {a b c d} {lappend ::log "rel $a,$b,$c,$d"}
bind .e <Motion> {rec motion %s %x %y}
bind .e <Enter> {rec enter %s %W}
bind .e <Leave> {rec leave %s}
bind .e <Configure> {rec conf %w %h}
bind .e <FocusIn> {rec focusin %s}
bind .e <Map> {rec map %s}
bind .e <MouseWheel> {rec wheel %D %s}
event generate .e <Key-a>
event generate .e <Key-a> -state 0x4
event generate .e <Key-Return>
event generate .e <Button-1> -x 10 -y 20 -button 1
event generate .e <ButtonRelease-1> -x 10 -y 20 -button 1
event generate .e <Motion> -x 12 -y 24
event generate .e <Enter>
event generate .e <Leave>
event generate .e <Configure> -width 100 -height 50
event generate .e <FocusIn>
event generate .e <Map>
event generate .e <MouseWheel> -delta -120
puts "BADOPT\t[catch {event generate .e <Key> -nonsense 1} m] $m"
grab set .e
puts "GRABS\t[grab status .e]|[grab current .e]"
grab release .e
puts "GRABR\t[grab status .e]"
foreach line $log { puts "EV|$line" }
exit 0
