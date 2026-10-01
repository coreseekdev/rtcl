proc p {} {return GLOBAL}
namespace eval tns {
    proc p {} {return NSLOCAL}
    puts "which=[namespace which -command p] call=[p]"
}
puts "outside=[p]"
