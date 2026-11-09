label .title -text "rtcl Tk on Android"
pack .title -padx 12 -pady 6
entry .name -width 20
pack .name -padx 12
button .go -text "Tap me" -command {
    .title configure -text "Tapped!"
}
pack .go -padx 12

image create photo demoimg -file /data/local/tmp/rtcl-demo.png
text .doc -height 5
.doc insert end "with embedded:"
.doc insert end "\n"
.doc image create end -image demoimg
.doc insert end "\n(inline above)"
.doc tag add hot 1.0 end
pack .doc -fill x -padx 12 -pady 4
