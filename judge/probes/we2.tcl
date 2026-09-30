proc boomq {} {error "boomq-msg"}
catch {puts "B1:[boomq]"}
puts "B1:[string map {\n |} $::errorInfo]"
catch {puts "B2:[set]"}
puts "B2:[string map {\n |} $::errorInfo]"
catch {puts "B3:[expr {1+}]"}
puts "B3:[string map {\n |} $::errorInfo]"
catch {puts "B4:[info level 0]"}
puts "B4:[string map {\n |} $::errorInfo]"
catch {puts "B5:[unset nosuchvar]"}
puts "B5:[string map {\n |} $::errorInfo]"
