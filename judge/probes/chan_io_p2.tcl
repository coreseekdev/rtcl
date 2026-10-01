foreach c {chan close flush fblocked fconfigure fileevent gets puts read seek tell eof pending socket open} {
  puts "$c=[info complete $c] [llength [info commands $c]]"
}
puts [info commands chan*]
