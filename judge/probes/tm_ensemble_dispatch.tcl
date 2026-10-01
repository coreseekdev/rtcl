namespace eval ::tmx {
    proc add {args} {return A}
    proc list {} {return L}
    proc remove {args} {return R}
    namespace export add list remove
    namespace ensemble create -command path
}
puts "p=[info commands ::tmx::path]"
puts "c1=[catch {::tmx::path foo} m1] <$m1>"
puts "c2=[catch {::tmx::path add} m2] <$m2>"
puts "c3=[::tmx::path list]"
