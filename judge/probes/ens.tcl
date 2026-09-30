namespace eval ns {
    namespace export x*
    proc x1 {} {}
    proc x2 {} {}
    namespace ensemble create
}
catch {ns zz} m; puts "A:$m"
catch {ns ?} m; puts "B:$m"
namespace delete ns
namespace eval ns {
    namespace ensemble create -map {a a}
}
catch {ns zz} m; puts "C:$m"
namespace ensemble configure ns -map {b b}
catch {ns zz} m; puts "D:$m"
puts "E:[namespace ensemble configure ns]"
namespace delete ns
catch {namespace ensemble create -command} m; puts "F:$m"
catch {namespace ensemble create -nosuchopt v} m; puts "G:$m"
catch {namespace ensemble configure} m; puts "H:$m"
catch {namespace ensemble configure nope} m; puts "I:$m"
namespace eval e2 { proc s1 {} {}; proc s2 {} {}; export s1 s2; namespace ensemble create }
puts "J:[namespace ensemble configure ::e2]"
catch {e2 s} m; puts "K:$m"
catch {e2 s1 extra args} m; puts "L:$m"
puts "M:[catch {e2} m2]; $m2"
namespace delete e2
