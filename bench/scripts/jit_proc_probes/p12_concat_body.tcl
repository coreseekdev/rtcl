proc p {s} {
  return "v=${s}x[expr {$s + 1}]"
}
puts [p 5]
