foreach c {digit integer alpha boolean space alnum upper lower ascii print control xdigit graph punct double list wideinteger} {
    puts "is $c {} => [string is $c {}]"
    puts "is $c -strict {} => [string is $c -strict {}]"
}
