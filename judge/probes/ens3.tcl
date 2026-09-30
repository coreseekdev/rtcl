namespace eval h { proc Hidden {} {}; namespace export {[a-z]*}; proc aa {} {} ; namespace ensemble create }
catch {h zz} m; puts "A:$m"
catch {h Hidden} m; puts "B:$m"
namespace delete h
namespace eval p2 { proc x {a b} {list $a-$b}; proc y {} {list Y}; namespace export *; namespace ensemble create -parameters {p1 p2} }
catch {p2 v1 v2 y} m; puts "C:$m"
catch {p2 y v1 v2} m; puts "D:$m"
namespace delete p2
namespace eval m1 {
    proc impl {args} {list IMPL $args}
    namespace export *
    namespace ensemble create -map {s1 {::m1::impl pre}}
}
catch {m1 s1 a b} m; puts "E:$m"
catch {m1 s} m; puts "F:$m"
namespace delete m1
namespace eval w1 { namespace export x; proc x {} {list XV}; proc y {} {list YV}; namespace ensemble create -subcommands {x y} }
puts "G:[w1 x]"
namespace delete w1
