proc show {label script} {
    set c [catch {uplevel 1 $script} r o]
    puts "$label: catchcode=$c r=$r o=$o"
}
catch {return hi} r o
puts "plain-return: catchcode=[catch {return hi} r o] r=$r o=$o"
catch {return -code return -level 1 hi} r o
puts "code-ret-lvl1: r=$r o=$o"
catch {return -code break -level 1} r o
puts "code-break-lvl1: r=$r o=$o"
catch {return -level 2 hi} r o
puts "lvl2: r=$r o=$o"
catch {return -level 0 hi} r o
puts "lvl0: r=$r o=$o"
catch {error boom} r o
puts "error: r=$r o=$o"
catch {expr {1+1}} r o
puts "ok: r=$r o=$o"
