# 空转（T11b 普查 2026-11-26）：两边死在行1：`variable w:::x 9` 报 can't define "w:::x": parent namespace doesn't exist，行2–8 未跑
namespace eval q {variable w:::x 9}
puts "exists-ns-qw=[namespace exists ::q::w]"
puts "c=[catch {set ::q::w::x} r] r=$r"
puts "vars-q=[info vars ::q::*]"
namespace eval q {puts "inside=[info vars]"}
namespace eval q {variable y::z 9}
puts "exists-ns-qy=[namespace exists ::q::y]"
puts "c2=[catch {set ::q::y::z} r2] r2=$r2"
