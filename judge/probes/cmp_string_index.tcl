# cmp_string_index.tcl — `string index` index-argument semantics.
# gen_compile 11.2: rtcl silently returns "" for an unparseable index where
# tclsh raises `bad index "bogus": must be integer?[+-]integer? or
# end?[+-]integer?` (yes, the ?[+-]? runs are literal in 8.6).
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

t si_plain   {string index abcde 0}
t si_last    {string index abcde 4}
t si_oob     {string index abcde 5}
t si_neg     {string index abcde -1}
t si_end     {string index abcde end}
t si_end0    {string index abcde end-0}
t si_endp1   {string index abcde end+1}
t si_endm10  {string index abcde end-10}
t si_bogus   {string index a bogus}
t si_empty   {string index a {}}
t si_plus    {string index abcde +2}
t si_sp      {string index abcde { 2}}
t si_hex     {string index abcde 0x2}
t si_oct     {string index abcde 010}
t si_wide    {string index abcde 0x100000000}
t si_bool    {string index abcde true}
t si_float   {string index abcde 1.5}
t si_inoper  {string index abcde 2+1}
t si_big     {string index abcde 99999999999999999999}
t si_endbig  {string index abcde end-99999999999999999999}
t si_argc    {string index a}
t si_argc2   {string index a 0 1}
# neighbours that share Tcl_GetIndexFromObj-style index parsing
t lr_bogus   {lrange {a b c} 0 bogus}
t ls_bogus   {lindex a bogus}
t sr_bogus   {string range abc bogus end}
t si_nul     {string index a\x00b 1}
t sl_nul     {string length foo\x00}
