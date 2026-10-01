namespace eval test_ns_1 { variable v 42 }
puts "code=[namespace eval test_ns_1 {namespace code {set v}}]"
puts "whichv=[namespace which -variable v]"
namespace eval test_ns_1 {puts "inside-v=[set v]"}
namespace eval test_ns_2 {proc namespace args {puts "SHADOW args=$args"}}
catch {namespace eval test_ns_2 [namespace eval test_ns_1 {namespace code {set v}}]} m
puts "c=[catch {set r} ]"
catch {namespace eval test_ns_2 [namespace eval test_ns_1 {namespace code {set v}}]} r
puts "r=$r"
