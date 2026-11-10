label .title -text "rtcl Tk"
pack .title -padx 12 -pady 6
entry .name -width 20
pack .name -padx 12
button .go -text "Tap me" -command {
    .title configure -text "Tapped!"
}
pack .go -padx 12

menu .m -tearoff 0
menu .m.file -tearoff 0
.m.file add command -label "New" -underline 0 -accelerator Ctrl+N -command {
    .title configure -text "New from menu"
}
.m.file add separator
.m.file add command -label "Exit" -command { destroy . }
.m add cascade -label "File" -menu .m.file -underline 0
. configure -menu .m

# The native dialogs (Win32 MessageBoxW and a template DialogBox here),
# self-dismissed by the verification run killing the process while up.
after 600 {
    tk_messageBox -type ok -title "About rtcl Tk" -message "rtcl native Tk on this platform" -icon information
    tk_dialog .pick "Choose" "Pick one of the buttons" "" 1 Yes No Cancel
}

image create photo demoimg -file {__DEMO_PNG__}
image create photo demoData -data {data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABgAAAAYCAYAAADgdz34AAAAJUlEQVR4nGOQS771n5aYYdSCUQtGLRi1YNSCUQtGLRi1YGhYAACyjErMPxGm9gAAAABJRU5ErkJggg==}
label .img -image demoData
pack .img -padx 12
text .doc -height 5
.doc insert end "with embedded:"
.doc insert end "\n"
.doc image create end -image demoimg
.doc insert end "\n(inline above)"
.doc tag add hot 1.0 end
pack .doc -fill x -padx 12 -pady 4
