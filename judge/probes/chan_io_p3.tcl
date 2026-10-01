proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r|EC=$::errorCode" }
t bare {chan}
t bare-arg {chan foo}
t create-1 {chan create}
t create-3 {chan create a b c}
t push-1 {chan push}
t push-3 {chan push a b c}
t pend-noargs {chan pending}
t pend-1 {chan pending input}
t pend-3 {chan pending input stdout x}
t pend-baddir {chan pending sideways stdout}
t pend-badchan {chan pending input gorp}
t names {chan names}
t cfg-eofchar-nul {chan configure stdout -eofchar \x00}
t cfg-eofchar-high {chan configure stdout -eofchar Ā}
t cfg-eofchar-list {chan configure stdout -eofchar [list \x27 \x80]}
t cfg-eofchar-ok {chan configure stdout -eofchar \x1a; chan configure stdout -eofchar {}}
t flush-stdin {flush stdin}
t puts-stdin {puts stdin hi}
t gets-stdout {gets stdout}
t read-stdout {read stdout}
t fblocked-file1000 {fblocked file1000}
t fblocked-stdout {fblocked stdout}
t fblocked-stdin {fblocked stdin}
t fblocked-noargs {fblocked}
t fblocked-2 {fblocked a b}
t eof-gorp {eof gorp}
t flush-gorp {flush gorp}
t flush-noargs {flush}
