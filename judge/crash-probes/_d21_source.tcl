# Source of judge/crash-probes/d21_interp_family.tcl: the `interp`
# command family, check by check, in the order its state is built up.
#   T <tag> {<script>}            — here: code, result, errorCode
#   C <tag> <path> {<script>}     — in a child: code, result, errorCode,
#                                  errorInfo (through the helper P below)
# `_gen_d21.tcl` runs every line under tclsh and writes the probe with
# tclsh's outputs as the expected values.
proc P {script} {
    set c [catch {uplevel 1 $script} r o]
    set e [catch {dict get $o -errorcode} e]
    set i [catch {dict get $o -errorinfo} i]
    list $c $r [expr {$e ? "" : $e}] [expr {$i ? "" : [string map {"\n" "^"} $i]}]
}

# ── paths ────────────────────────────────────────────────────────────────
T p-create {interp create}
T p-create2 {interp create}
T p-exists {list [interp exists] [interp exists interp0] [interp exists interp1] [interp exists nope] [interp exists {}]}
T p-slaves {lsort [interp slaves]}
T p-nested-create {interp create {interp0 g}}
T p-nested-exists {list [interp exists {interp0 g}] [interp exists g] [interp exists interp0]}
T p-nested-slaves {lsort [interp slaves interp0]}
T p-nested-eval {interp eval {interp0 g} {set x 1}}
T p-nested-path-cmd {interp eval interp0 {g eval {expr 2 * 3}}}
T p-badpath {interp eval nope {set x 1}}
T p-badpath2 {interp eval {interp0 nope} {set x 1}}
T p-self {interp eval {} {expr 1 + 1}}
T p-self-issafe {interp issafe}
T p-issafe-nested {list [interp issafe interp0] [interp issafe {interp0 g}] [interp issafe nope]}
T p-ns-name {interp create ::a::b::c}
T p-ns-exists {list [namespace exists ::a] [namespace exists ::a::b] [interp exists ::a::b::c]}
T p-ns-eval {::a::b::c eval {expr 4 + 4}}
T p-ns-delete {interp delete ::a::b::c}
T p-ns-after {list [namespace exists ::a] [interp exists ::a::b::c]}
T p-children {lsort [interp children interp0]}
T p-aliases-nested {lsort [interp aliases {interp0 g}]}
T p-hidden-nested {interp hidden {interp0 g}}
T p-reclimit-nested {interp recursionlimit {interp0 g}}

# ── create ───────────────────────────────────────────────────────────────
T c-opt-after {interp create sa -safe}
T c-opt-issafe {interp issafe sa}
T c-opt-twice {interp create sb -safe -safe}
T c-opt-dashdash {interp create -- sc}
T c-opt-dashdash-name {interp create -- -sd}
T c-opt-bad {interp create -untrusted se}
T c-opt-two {interp create x y}
T c-opt-existing {interp create sa}
T c-opt-existing-cmd {proc mycmd {} {return 1}; interp create mycmd}
T c-opt-over-proc {info procs mycmd}
T c-default-after-cmd {proc interp9 {} {}; interp create}
T c-delete-both {interp delete sb sc -sd mycmd interp9}
T c-bare {interp}
T c-ev {interp ev sa {expr 5}}
T c-e {interp e}
T c-al {interp al}
T c-bogus {interp bogus}
interp delete sa

# ── delete ───────────────────────────────────────────────────────────────
T d-self {interp delete {}}
T d-missing {interp delete nope}
T d-none {interp delete}
T d-multi {interp create d1; interp create d2; interp create d3; interp delete d1 nope d3}
T d-multi-after {list [interp exists d1] [interp exists d2] [interp exists d3]}
T d-cmd-gone {catch {d1 eval x} m; set m}
T d-nested {interp create d4; interp eval d4 {interp create e}; interp delete {d4 e}}
T d-nested-after {list [interp exists {d4 e}] [lsort [interp slaves d4]]}
T d-outer {interp delete d4}
T d-rename-away {interp create d5; interp alias d5 w {} list; rename d5 {}}
T d-rename-after {list [interp exists d5] [info commands d5]}
T d-inflight {interp create d6; interp alias d6 w {} interp delete d6; interp eval d6 w}
T d-inflight2 {interp exists d6}

# ── hide / expose / invokehidden ─────────────────────────────────────────
interp create h1
T h-hide {interp hide h1 exec}
T h-hidden {lsort [interp hidden h1]}
T h-call {interp eval h1 {catch {exec echo hi} m; set m}}
T h-call-ec {interp eval h1 {catch {exec echo hi} m o; dict get $o -errorcode}}
T h-info {interp eval h1 {info commands ex*}}
T h-twice {interp hide h1 exec}
T h-twice-ec {catch {interp hide h1 exec} m o; dict get $o -errorcode}
T h-nosuch {interp hide h1 nosuchcmd}
T h-nosuchpath {interp hide nope exec}
T h-empty {interp hide h1 {}}
T h-invoke {interp invokehidden h1 exec echo hi}
T h-invoke-hidden-name {interp eval h1 {catch {interp invokehidden h1 exec echo x} m; set m}}
T h-invoke-bad {interp invokehidden h1 nosuch cmd}
T h-invoke-arity {interp invokehidden h1 exec}
T h-invoke-wa {catch {interp invokehidden h1 exec} m o; list $m [dict get $o -errorcode]}
T h-invoke-hidden-set {interp hide h1 set; interp invokehidden h1 set zz 3}
T h-hidden-set {interp eval h1 {catch {set a 1} m; set m}}
T h-hidden-set-ec {interp eval h1 {catch {set a 1} m o; dict get $o -errorcode}}
T h-hidden-info {interp eval h1 {info commands set}}
T h-expose {interp expose h1 set}
T h-expose-again {interp expose h1 set}
T h-expose-existing {interp hide h1 exec; interp expose h1 exec puts}
T h-expose-existing-ec {catch {interp expose h1 exec puts} m o; dict get $o -errorcode}
T h-expose-new {interp expose h1 exec myexec}
T h-expose-new-call {interp eval h1 {myexec echo via-newname}}
T h-expose-new-wa {interp eval h1 {catch {myexec} m o; dict get $o -errorinfo}}
T h-hidden-after {lsort [interp hidden h1]}
T h-expose-ns {interp hide h1 clock; interp expose h1 clock ::x}
T h-expose-ns-ec {catch {interp expose h1 clock ::x} m o; dict get $o -errorcode}
T h-hide-token-ns {interp hide h1 clock a::b}
T h-hide-token {interp hide h1 clock ck}
T h-hidden-token {lsort [interp hidden h1]}
T h-invoke-token {interp invokehidden h1 ck format 0 -format %Y}
T h-hide-token-taken {interp hide h1 list lk}
T h-hide-token-taken-ec {catch {interp hide h1 list lk} m o; dict get $o -errorcode}
T h-invoke-ns {interp invokehidden h1 -namespace ::nos ck format 0 -format %Y}
T h-invoke-global {interp invokehidden -global h1 ck format 0 -format %Y}
T h-invoke-dashdash {interp invokehidden -- h1 ck format 0 -format %Y}
T h-invoke-badopt {interp invokehidden h1 -bogus x}
T h-invoke-badopt2 {interp invokehidden h1 -bogus}
T h-invoke-badopt-ec {catch {interp invokehidden h1 -bogus} m o; dict get $o -errorcode}
T h-invoke-ns-missing {interp invokehidden h1 -namespace}
T h-proc {interp eval h1 {proc hp {x} {return "hp:$x"}}; interp hide h1 hp}
T h-invoke-proc {interp invokehidden h1 hp 7}
T h-invoke-proc-wa {catch {interp invokehidden h1 hp} m o; set m}
T h-expose-proc {interp expose h1 hp; interp eval h1 {hp 8}}
T h-ensemble {interp hide h1 info; interp invokehidden h1 info commands set}
T h-hide-interp {interp hide h1 interp; interp eval h1 {catch {interp aliases} m; set m}}
T h-invoke-interp {interp invokehidden h1 interp hidden}
T h-expose-interp {interp invokehidden h1 interp expose h1 interp}
T h-rename-hidden {interp eval h1 {catch {rename ck other} m; set m}}
T h-compiled-before {interp eval h1 {proc hs {} {set s 1; return $s}}; interp eval h1 hs}
T h-hide-set {interp hide h1 set}
T h-compiled-after {interp eval h1 {catch {hs} m o; set m}}
T h-compiled-new {interp eval h1 {proc hs2 {} {set s 2; return $s}}; catch {hs2} m o; set m}
T h-expose-set {interp expose h1 set}
T h-compiled-back {interp eval h1 hs}
T h-compiled-back2 {interp eval h1 hs2}
T h-expose-list {interp expose h1 list lk}
interp delete h1

# ── safe ─────────────────────────────────────────────────────────────────
interp create sv -safe
T s-issafe {interp issafe sv}
T s-hidden-common {lsort [lmap c [interp hidden sv] {expr {[string match tcl:* $c] ? "" : $c}}]}
T s-aliases {lsort [interp aliases sv]}
T s-exec {interp eval sv {catch {exec echo hi} m; set m}}
T s-open {interp eval sv {catch {open /etc/hostname} m; set m}}
T s-file {interp eval sv {catch {file exists /etc/hostname} m; set m}}
T s-cd {interp eval sv {catch {cd /} m; set m}}
T s-puts {interp eval sv {catch {puts hi} m; set m}}
T s-expr {interp eval sv {expr 1 + 1}}
T s-source {interp eval sv {catch {source /etc/hostname} m; set m}}
T s-set {interp eval sv {set x 1}}
T s-glob {interp eval sv {catch {glob /*} m; set m}}
T s-env {interp eval sv {catch {array names env} m; set m}}
T s-env2 {interp eval sv {catch {set env(PATH)} m; set m}}
T s-exit {interp eval sv {catch {exit} m o; dict get $o -errorcode}}
T s-load {interp eval sv {catch {load x} m; set m}}
T s-socket {interp eval sv {catch {socket 127.0.0.1 1} m; set m}}
T s-pwd {interp eval sv {catch {pwd} m; set m}}
T s-fconfig {interp eval sv {catch {fconfigure stdin} m; set m}}
T s-version {interp eval sv {info tclversion}}
T s-platform {lsort [interp eval sv {array names tcl_platform}]}
T s-lib {interp eval sv {list [info exists tcl_library] [info exists tcl_pkgPath] [info exists env]}}
T s-clock {interp eval sv {clock format 0 -format %Y}}
T s-math {interp eval sv {list [::tcl::mathfunc::min 2 9] [::tcl::mathfunc::max 2 9]}}
T s-interp-exists {interp eval sv {info commands interp}}
T s-create-child {interp eval sv {interp create k}}
T s-child-safe {interp eval sv {interp issafe k}}
T s-child-exec {interp eval sv {catch {k eval {exec ls}} m; set m}}
T s-hide {interp eval sv {catch {interp hide k set} m o; list $m [dict get $o -errorcode]}}
T s-expose {interp eval sv {catch {interp expose k exec} m o; list $m [dict get $o -errorcode]}}
T s-expose2 {interp eval sv {catch {interp expose k exec exec} m o; list $m [dict get $o -errorcode]}}
T s-invokehidden {interp eval sv {catch {interp invokehidden k clock seconds} m o; list $m [dict get $o -errorcode]}}
T s-marktrusted {interp eval sv {catch {interp marktrusted k} m o; list $m [dict get $o -errorcode]}}
T s-reclimit-set {interp eval sv {catch {interp recursionlimit k 5} m o; list $m [dict get $o -errorcode]}}
T s-reclimit-get {interp eval sv {interp recursionlimit k}}
T s-alias-into-self {interp eval sv {interp alias k a {} set ::kv 1}; interp eval sv {catch {k eval a} m o; set m}}
T s-kv {interp eval sv {set ::kv}}
T s-alias-out {interp eval sv {catch {interp alias k b {} nosuchcmd} m o; list $m [dict get $o -errorcode]}}
T s-alias-out-call {interp eval sv {catch {k eval b} m o; list $m [dict get $o -errorcode]}}
T s-alias-badmaster {interp eval sv {catch {interp alias k c {} nosuch} m o; list $m [dict get $o -errorcode]}}
T s-delete-child {interp eval sv {catch {interp delete k} m o; list $m [dict get $o -errorcode]}}
T s-exists {interp eval sv {list [interp exists] [interp exists k] [interp exists nope]}}
T s-slaves {interp eval sv {lsort [interp slaves]}}
T s-aliases-self {interp eval sv {lsort [interp aliases]}}
T s-hidden-self {interp eval sv {llength [interp hidden]}}
T s-limit {interp eval sv {catch {interp limit} m o; set m}}
T s-marktrusted-from-master {interp marktrusted sv}
T s-after {list [interp issafe sv] [interp eval sv {catch {exec echo hi} m; set m}]}
T s-create-trusted-child {interp eval sv {interp create k2; interp issafe k2}}
T s-uptail {interp eval sv {catch {uplevel #0 {interp hide k2 set}} m o; list $m [dict get $o -errorcode]}}
interp delete sv

# ── aliases ──────────────────────────────────────────────────────────────
interp create s1
interp create s2
T a-empty-master {interp alias s1 a1 {} list}
T a-query {interp alias s1 a1}
T a-call {interp eval s1 {a1 1 2}}
T a-again {interp alias s1 a1 {} concat}
T a-query2 {interp alias s1 a1}
T a-call2 {interp eval s1 {a1 1 2}}
T a-in-path {interp alias s1 a2 s1 list}
T a-in-path-call {interp eval s1 {a2 1 2}}
T a-in-path-query {interp alias s1 a2}
T a-in-path-target {interp target s1 a2}
T a-cross-target {interp target s1 a1}
T a-target-missing {interp target s1 nosuch}
T a-del-inpath {interp alias s1 a2 {}}
T a-del-missing {interp alias s1 a2 {}}
T a-del-masterpath {interp alias s1 a1 s1}
T a-one-extra {interp alias s1 a1 x}
T a-two-extra {interp alias s1 a1 x y}
T a-empty-name {interp alias s1 {} {} list}
T a-missing-path {interp alias nope a b}
T a-bad-master {interp alias s1 a9 nope list}
T a-into-parent {interp alias {} pa {} list; pa 7 8}
T a-into-parent-query {interp alias {} pa}
T a-into-parent-aliases {lsort [interp aliases]}
T a-into-parent-target {interp target {} pa}
T a-into-parent-del {interp alias {} pa {}}
T a-parent-into-child {interp alias {} pc s1 set; pc x 3}
T a-parent-into-child-r {interp eval s1 {set x}}
T a-parent-into-child-q {interp alias {} pc}
T a-parent-into-child-t {interp target {} pc}
T a-parent-into-child-t2 {catch {interp target s1 pc} m o; set m}
T a-parent-into-child-err {catch {pc nosuchvar} m o; list $m [dict get $o -errorcode] [dict get $o -errorinfo]}
T a-parent-into-child-wa {catch {pc} m o; list $m [dict get $o -errorinfo]}
T a-parent-into-child-ret {catch {pc interp alias} m o; set m}
T a-parent-into-child-break {catch {pc break} m o; list $m [dict get $o -errorcode]}
T a-parent-into-child-code {set ::errorCode}
T a-sibling {interp alias s1 ab s2 set y 5}
T a-sibling-call {interp eval s1 {ab}}
T a-sibling-r {interp eval s2 {set y}}
T a-sibling-q {interp alias s1 ab}
T a-sibling-t {interp target s1 ab}
T a-sibling-t2 {catch {interp target s2 ab} m o; set m}
T a-sibling-t3 {interp eval s1 {catch {interp target {} ab} m o; set m}}
T a-sibling-err {interp eval s1 {catch {ab nosuchvar} m o; list $m [dict get $o -errorcode] [dict get $o -errorinfo]}}
T a-error {interp alias s1 ae {} error aerr; interp eval s1 {catch {ae} m o; list $m [dict get $o -errorcode] [dict get $o -errorinfo]}}
T a-error-parent-code {set ::errorCode}
T a-error-parent-info {catch {interp eval s1 ae} m o; dict get $o -errorinfo}
T a-error-parent-ec {catch {interp eval s1 ae} m o; dict get $o -errorcode}
T a-error-prefix {interp alias s1 ae2 {} error pre {A B}; interp eval s1 {catch {ae2} m o; list $m [dict get $o -errorcode] [dict get $o -errorinfo]}}
T a-error-in-proc {interp eval s1 {proc q {} {ae}; catch {q} m o; dict get $o -errorinfo}}
T a-error-stack {interp eval s1 {proc q2 {} {ae}; catch {q2} m o; dict get $o -errorstack}}
T a-unknown-target {interp alias s1 au {} nosuchcmd; interp eval s1 {catch {au x} m o; list $m [dict get $o -errorcode] [dict get $o -errorinfo]}}
T a-bad-target-arg {interp alias s1 aq {} llength; interp eval s1 {catch {aq} m o; list $m [dict get $o -errorinfo]}}
T a-nsname {interp alias s1 {::ns::al} {} list}
T a-nsname-call {interp eval s1 {catch {ns::al 1} m o; set m}}
T a-nsname-call2 {interp eval s1 {catch {{::ns::al} 1} m o; set m}}
T a-nsname-q {interp alias s1 {::ns::al}}
T a-ns-target {interp alias s1 nt {} set ::nsv; interp eval s1 {nt 4}}
T a-ns-target-r {set ::nsv}
T a-ns-var-err {interp eval s1 {catch {nt} m o; list $m [dict get $o -errorcode]}}
T a-prefix {interp alias s1 pf {} list A {b c}; interp eval s1 {pf "d e" {f g}}}
T a-prefix-literal {interp alias s1 pl {} set p "[expr 1 + 1] \$x"; interp eval s1 {pl}}
T a-prefix-literal-r {interp eval s1 {set p}}
T a-replace-proc {interp eval s1 {proc rp {x} {return "rp:$x"}}; interp alias s1 rp {} list}
T a-replace-proc-call {interp eval s1 {rp 3}}
T a-replace-alias {interp alias s1 rp s1 concat}
T a-replace-alias-call {interp eval s1 {rp 3 4}}
T a-recreate {interp alias s1 rc {} list; interp alias s1 rc {} concat}
T a-recreate-call {interp eval s1 {rc 3 4}}
T a-list {lsort [interp aliases s1]}
T a-hidden-target {interp hide s1 set; interp eval s1 {catch {a1 1} m o; set m}}
T a-call-into-hidden {interp alias s1 ah s1 set hh 1; interp eval s1 {catch {ah} m o; set m}}
T a-invokehidden-alias {interp invokehidden s1 a1 5}
T a-expose-set {interp expose s1 set}
T a-expose-set2 {interp eval s1 {catch {set hh} m o; set m}}
T a-hide-alias {interp hide s1 a1}
T a-hidden-list {lsort [interp hidden s1]}
T a-aliases-after-hide {lsort [interp aliases s1]}
T a-hidden-alias-call {interp eval s1 {catch {a1 1} m o; set m}}
T a-invoke-hidden-alias {interp invokehidden s1 a1 6}
T a-expose-alias {interp expose s1 a1}
T a-exposed-alias-call {interp eval s1 {a1 7}}
T a-rename-alias-in-child {interp eval s1 {rename a1 a1r}}
T a-renamed-call {interp eval s1 {a1r 8}}
T a-renamed-aliases {lsort [interp aliases s1]}
T a-renamed-path {rename s1 s1r}
T a-renamed-path-eval {s1r eval {a1r 10}}
T a-renamed-path-exists {list [interp exists s1] [interp exists s1r]}
T a-renamed-path-interp {catch {interp eval s1 {a1r 11}} m o; set m}
T a-renamed-path-alias {catch {interp alias s1 newa {} list} m o; set m}
T a-renamed-path-alias-call {s1r eval {newa 12}}
T a-renamed-path-hide {s1r hide a1r; lsort [s1r hidden]}
T a-renamed-path-expose {s1r expose a1r}
T a-renamed-slaves {lsort [interp slaves]}
T a-renamed-back {rename s1r s1}
T a-renamed-back-eval {s1 eval {newa 13}}

# ── aliases: a grandchild ────────────────────────────────────────────────
T a-deep-create {interp eval s1 {interp create g}}
T a-deep-alias-top {interp alias {s1 g} ga {} set ::gg 8}
T a-deep-alias-call {interp eval {s1 g} {ga}}
T a-deep-alias-r {set ::gg}
T a-deep-alias-call2 {interp eval s1 {catch {ga} m o; set m}}
T a-deep-alias-q {interp alias {s1 g} ga}
T a-deep-alias-q2 {interp eval s1 {interp alias g ga}}
T a-deep-alias-t {interp target {s1 g} ga}
T a-deep-alias-t2 {interp eval s1 {catch {interp target g ga} m o; set m}}
T a-deep-alias-t3 {interp eval {s1 g} {catch {interp target {} ga} m o; set m}}
T a-deep-into-s1 {interp alias {s1 g} gb s1 set ggb 3}
T a-deep-into-s1-call {interp eval {s1 g} {gb}}
T a-deep-into-s1-r {interp eval s1 {set ggb}}
T a-deep-into-s1-q {interp alias {s1 g} gb}
T a-deep-into-s1-t {interp eval s1 {catch {interp target g gb} m o; set m}}
T a-deep-into-self {interp alias {s1 g} gc {s1 g} set ggc 4}
T a-deep-into-self-call {interp eval {s1 g} {gc}}
T a-deep-into-self-r {interp eval {s1 g} {set ggc}}
T a-deep-into-sibling {interp create s3; interp alias {s1 g} gd s3 set q 1}
T a-deep-into-sibling-call {interp eval {s1 g} {gd}; interp eval s3 {set q}}
T a-deep-into-sibling-t {interp eval s1 {catch {interp target g gd} m o; set m}}
T a-deep-from-sibling {interp eval s1 {catch {interp alias g ge s2 set q 2} m o; set m}}
T a-deep-make-from-inside {interp eval s1 {interp alias g gf s2 set q 3}}
T a-deep-make-from-inside-call {catch {interp eval {s1 g} {gf}} m o; set m}
T a-deep-aliases {lsort [interp aliases {s1 g}]}
T a-deep-delete-sibling {interp delete s2; lsort [interp aliases {s1 g}]}
T a-deep-after-delete {interp eval {s1 g} {catch {gb} m o; set m}}
T a-deep-after-delete-aliases {lsort [interp aliases s1]}
C a-deep-err {s1} {interp eval g {proc p {} {gb 1}; catch {p} m o; dict get $o -errorinfo}}
C a-deep-err2 {s1} {interp alias g gx {} error deep; interp eval g {catch {gx} m o; list $m [dict get $o -errorinfo]}}
C a-deep-err3 {s1} {interp alias g gx {} error deep; interp eval g {catch {gx} m o}; dict get $o -errorinfo}

# ── aliases: loops ───────────────────────────────────────────────────────
T l-self {interp alias {} z {} z}
T l-self-ec {set ::errorCode}
T l-self-call {catch {z} m o; set m}
T l-2 {interp create l2; interp alias l2 x {} x}
T l-2b {interp alias l2 x {} x2}
T l-3 {interp create l3; interp alias l3 x l3 x}
T l-4 {interp create l4; interp alias l4 x {} y}
T l-4b {interp alias {} y l4 x}
T l-4c {interp alias {} y l4 z}
T l-5 {interp create l5; interp alias l5 x l5 y}
T l-5b {interp alias l5 y l5 x}
T l-6 {interp alias {} l5z l5 q}
T l-6b {interp alias l5 q {} l5z}
T l-7 {interp create m1; interp create m2; interp alias m1 a m2 b}
T l-7b {interp alias m2 b m1 a}
T l-8 {interp create l8; interp alias l8 x {} l8y}
T l-8b {interp alias {} l8y l8 x}
T l-9 {interp create l9; interp alias l9 x {} r2; rename r2 x}
T l-9-ec {set ::errorCode}
T l-9-call {catch {interp eval l9 x} m o; set m}
T l-10 {interp alias {} r3 l9 x}
T l-10b {rename r3 l9x}
T l-10c {rename l9x r3}

# ── aliases: deletions ───────────────────────────────────────────────────
interp create e1
interp create e2
T x-1 {interp alias e1 t d2 set q 1}
T x-2 {interp alias e2 u d1 set q 2}
T x-3 {interp alias e1 v {} set q 3}
T x-4 {interp delete e2; lsort [interp aliases e1]}
T x-5 {interp eval e1 {catch {t} m o; set m}}
T x-6 {catch {interp alias e1 v {} d2 set} m o; set m}
T x-7 {interp delete e1; interp aliases}
T x-8 {interp alias {} w {} set q 4; interp delete {}}
T x-9 {interp create e3; interp alias {} w2 d3 set q 5; interp delete e3; interp aliases}
T x-10 {catch {w2} m o; set m}
T x-11 {interp create e4; interp alias e4 wa {} interp delete e4; interp eval e4 wa}
T x-12 {interp exists e4}
T x-13 {interp delete e4}
T x-14 {catch {w} m o; set m}
puts DONE
