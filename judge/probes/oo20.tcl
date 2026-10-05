# 1. memoized MISS then object created with that name
set r1 [catch {ghost x} e1]
oo::class create G { method x {} { return alive } }
G create ghost
set r2 [ghost x]
# 2. destroy then resolve again (memoized HIT must die)
ghost destroy
set r3 [catch {ghost x} e3]
# 3. recreate same name as a DIFFERENT class
oo::class create H { method x {} { return h-ver } }
H create ghost
set r4 [ghost x]
# 4. qualified vs bare of the same object
set r5 [::ghost x]
# 5. fresh object after prior memo fills
oo::class create K1 { method m {} { return k1 } }
K1 create kobj
set r6 [kobj m]
set r7 [kobj m]
set r8 [catch {nosuchobj m} e8]
list $r1:$e1 $r2 $r3:$e3 $r4 $r5 $r6 $r7 $r8
puts [list $r1:$e1 $r2 $r3:$e3 $r4 $r5 $r6 $r7 $r8]
