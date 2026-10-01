proc B {} {return bproc}
namespace eval nn {puts [namespace which B]}
puts [namespace eval nn {namespace which -command B}]
