namespace eval ns { namespace ensemble create -map {a a} -prefixes 0 }
catch {ns zz} m; puts "A:$m"
catch {ns a} m; puts "A2:$m"
puts "B:[namespace ensemble configure ns -unknown]"
namespace ensemble configure ns -unknown bar
puts "C:[namespace ensemble configure ns -unknown]"
namespace delete ns
namespace eval s2 { namespace export {[a-z]*}; proc aa {} {puts ran-aa}; proc ab {} {}; namespace ensemble create }
catch {s2 a} m; puts "D:$m"
catch {s2 aa} m; puts "E:$m"
namespace ensemble configure s2 -subcommands {aa}
catch {s2 ab} m; puts "F:$m"
catch {s2 zz} m; puts "G:$m"
puts "H:[namespace ensemble configure s2 -subcommands]"
namespace delete s2
namespace eval p1 { proc x {para} {list got $para}; namespace export *; namespace ensemble create -parameters {p1} }
puts "I:[info command p1]"
catch {p1 bar x} m; puts "J:$m"
catch {p1 x bar} m; puts "K:$m"
namespace delete p1
namespace eval u1 { namespace ensemble create -command ::u1::ens -unknown {h1 pre} }
proc h1 {args} { return "H:[join $args ,]" }
puts "L:[u1::ens zzz m1 m2]"
rename u1::ens {}
