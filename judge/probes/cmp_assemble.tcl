# cmp_assemble.tcl — tcl::unsupported::assemble semantics for the
# gen_assemble-52.1 subset: push / invokeStk / pop / jump+label /
# beginCatch+endCatch, and the "push 1" (should-be pushReturnCode) quirk.
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

t a_push      {tcl::unsupported::assemble {push 1}}
t a_push_str  {tcl::unsupported::assemble {push hello}}
t a_push2     {tcl::unsupported::assemble {push a; push b}}
t a_over      {tcl::unsupported::assemble {push 1; over 0}}
t a_pop       {tcl::unsupported::assemble {push 1; pop}}
t a_swap      {tcl::unsupported::assemble {push a; push b; swap; pop}}
t a_label_jmp {tcl::unsupported::assemble {jump @end; push skipped; label @end; push done}}
t a_catch_ok  {tcl::unsupported::assemble {beginCatch @c; push 1; pop; endCatch; push ok; label @c; push bad}}
t a_catch_err {tcl::unsupported::assemble {beginCatch @c; error boom; pop; endCatch; push ok; label @c; push caught}}
t a_invoke    {tcl::unsupported::assemble {push string; push length; push abc; invokeStk 2; invokeStk 2}}
t a_invoke_rep {tcl::unsupported::assemble {
    beginCatch @badLabel
    push error
    push testing
    invokeStk 2
    pop
    push 0
    jump @okLabel
    label @badLabel
    push 1
    label @okLabel
    endCatch
    pop
}}
t a_bad_instr {tcl::unsupported::assemble {nosuchinstruction}}
t a_bad_label {tcl::unsupported::assemble {jump @nowhere}}
t a_dup_label {tcl::unsupported::assemble {label @x; label @x}}
t a_empty     {tcl::unsupported::assemble {}}
t a_getbc     {tcl::unsupported::getbytecode script {tcl::unsupported::assemble {push 1}}}
