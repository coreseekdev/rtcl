set acc 0
if [catch {incr acc []} msg] { puts "rc=$msg code=$errorCode" }
