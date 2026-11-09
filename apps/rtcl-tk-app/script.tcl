label .title -text "rtcl Tk on Android"
pack .title -padx 12 -pady 6
entry .name -width 20
pack .name -padx 12
button .go -text "Tap me" -command {
    .title configure -text "Tapped!"
}
pack .go -padx 12
