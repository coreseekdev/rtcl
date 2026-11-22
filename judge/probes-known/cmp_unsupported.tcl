# cmp_unsupported.tcl — tcl::unsupported::disassemble / getbytecode matrix
# gen_compile 18.x cluster: every corpus failing form plus arg-validation
# neighbours, so an implementer gets exact oracle wording without re-probing.
proc t {label script} { set c [catch {uplevel 1 $script} r]; puts "$label|$c|$r" }

# --- type validation -----------------------------------------------------
t type_none    {tcl::unsupported::disassemble}
t type_bogus   {tcl::unsupported::disassemble bogus}
t type_bogus2  {tcl::unsupported::disassemble bogus x y}
t type_list    {tcl::unsupported::getbytecode}
t gtype_bogus  {tcl::unsupported::getbytecode bogus}
t d_oo_prefix  {::tcl::unsupported::disassemble proc nosuchproc}
t g_oo_prefix  {::tcl::unsupported::getbytecode proc nosuchproc}

# --- proc form (corpus 18.7 / 18.27) --------------------------------------
t d_proc_miss  {tcl::unsupported::disassemble proc nosuchproc}
t g_proc_miss  {tcl::unsupported::getbytecode proc nosuchproc}
t d_proc_noarg {tcl::unsupported::disassemble proc}
t g_proc_noarg {tcl::unsupported::getbytecode proc}
t d_proc_extra {tcl::unsupported::disassemble proc nosuchproc junk}
t g_proc_extra {tcl::unsupported::getbytecode proc nosuchproc junk}
t d_proc_alias {tcl::unsupported::disassemble proc nosuchproc}
t d_proc_cmd   {tcl::unsupported::disassemble proc puts}
t g_proc_cmd   {tcl::unsupported::getbytecode proc puts}

# --- method form (corpus 18.12 / 18.14 / 18.32 / 18.34) -------------------
t d_meth_miss  {tcl::unsupported::disassemble method nosuchclass foo}
t g_meth_miss  {tcl::unsupported::getbytecode method nosuchclass foo}
t d_meth_oo    {tcl::unsupported::disassemble method oo::object nosuchmethod}
t g_meth_oo    {tcl::unsupported::getbytecode method oo::object nosuchmethod}
t d_meth_1arg  {tcl::unsupported::disassemble method nosuchclass}
t d_meth_0arg  {tcl::unsupported::disassemble method}
t d_meth_obj   {tcl::unsupported::disassemble method oo::object}
t g_meth_1arg  {tcl::unsupported::getbytecode method nosuchclass}

# --- objmethod form (corpus 18.17 / 18.18 / 18.37 / 18.38) ----------------
t d_objm_miss  {tcl::unsupported::disassemble objmethod nosuchobject foo}
t g_objm_miss  {tcl::unsupported::getbytecode objmethod nosuchobject foo}
t d_objm_oo    {tcl::unsupported::disassemble objmethod oo::object nosuchmethod}
t g_objm_oo    {tcl::unsupported::getbytecode objmethod oo::object nosuchmethod}
t d_objm_1arg  {tcl::unsupported::disassemble objmethod nosuchobject}

# --- constructor form (corpus 18.41 / 18.46) ------------------------------
t d_ctor_miss  {tcl::unsupported::disassemble constructor nosuchclass}
t g_ctor_miss  {tcl::unsupported::getbytecode constructor nosuchobject}
t d_ctor_0arg  {tcl::unsupported::disassemble constructor}
t d_ctor_oo    {tcl::unsupported::disassemble constructor oo::object}

# --- destructor form (corpus 18.51 / 18.56) -------------------------------
t d_dtor_miss  {tcl::unsupported::disassemble destructor nosuchclass}
t g_dtor_miss  {tcl::unsupported::getbytecode destructor nosuchobject}
t d_dtor_oo    {tcl::unsupported::disassemble destructor oo::object}

# --- lambda / script forms (context for implementer) ----------------------
t d_lambda_0   {tcl::unsupported::disassemble lambda}
t d_lambda_ok  {tcl::unsupported::disassemble lambda {{} {set x 1}}}
t g_lambda_ok  {tcl::unsupported::getbytecode lambda {{} {set x 1}}}
t d_script_ok  {tcl::unsupported::disassemble script {set x 1}}
t g_script_ok  {tcl::unsupported::getbytecode script {set x 1}}
t g_script_ea  {tcl::unsupported::getbytecode script {set x 1} extra}
t d_script_ea  {tcl::unsupported::disassemble script {set x 1} extra}
