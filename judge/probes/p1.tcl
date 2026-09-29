proc foo {} {}
trace add command foo rename traceCommand
puts [trace info command foo]
