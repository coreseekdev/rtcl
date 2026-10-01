namespace eval nsx { proc q {} {return 1} }
namespace eval nsx { q; puts "clean exists=[namespace exists ::nsx]" }
# now with an intermediate delete of a DIFFERENT ns that shares a var link
namespace eval test_ns_var { variable foo 2 }
proc pp {} { variable ::test_ns_var::foo; namespace delete ::test_ns_var }
pp
namespace eval test_ns_var { proc r {} {return 1} }
namespace eval test_ns_var { r; puts "poison exists=[namespace exists ::test_ns_var]" }
