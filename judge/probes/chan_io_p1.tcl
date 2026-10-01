proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }
t chan-bare {chan}
t chan-create0 {chan create}
t chan-create3 {chan create a b c}
t chan-push0 {chan push}
t chan-push3 {chan push a b c}
t eofchar-a {chan configure stdout -eofchar Ā}
t eofchar-nul {chan configure stdout -eofchar \x00}
t eofchar-list {chan configure stdout -eofchar [list \x27 \x80]}
t pend-in-stdout {chan pending input stdout}
t pend-in-stdin {chan pending input stdin}
t pend-out-stdin {chan pending output stdin}
t pend-out-stdout {chan pending output stdout}
t flush-stdin {flush stdin}
t fblocked-file1000 {fblocked file1000}
t fblocked-stdout {fblocked stdout}
t fblocked-stdin {fblocked stdin}
t eof-gorp {list [catch {eof gorp} m] $m $::errorCode}
