namespace eval test_ns_export {
    variable version 1.0
    proc exported1 {} { return "exported1 in test_ns_export" }
    proc exported2 {} { return "exported2 in test_ns_export" }
    namespace export exported*
}
namespace eval test_ns_import_empty {
    namespace import ::test_ns_export::*
    puts "IMP<<[lsort [namespace import]]>>"
    namespace delete [namespace current]
}
puts "done"
