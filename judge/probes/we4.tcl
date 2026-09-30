catch {puts "C1:[info level 0]"}
puts "C1:[string map {\n |} $::errorInfo]"
catch {unset zz} ; set ::errorInfo ""
namespace eval nq { puts "C2:[info level 0]" }
