catch {lsort -integer {09 8}} m
puts "code=<[set ::errorCode]>"
catch {error foo}
puts "code2=<[set ::errorCode]>"
