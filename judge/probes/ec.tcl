catch {error foo} m
puts "code=<[set ::errorCode]>"
puts "exists=[info exists ::errorCode]"
