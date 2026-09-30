puts [string match {[a-z]*} "aa"]
puts [string match {[a-z]*} "Ab"]
puts [string match {[abc]x} "bx"]
puts [string match {[a-c]x} "bx"]
puts [string match {[^a]x} "bx"]
puts [string match {[!a]x} "bx"]
puts [string match {a[} "a["]
puts [string match {a[b} "a[b"]
puts [string match {[]x]y} "]y"
puts [string match {[a-]z} "-z"]
puts [string match {[\]]z} "]z"
puts [string match {[a\]z} "az"
puts [info patchlevel]
