// crash-wasm: the crash-probe classes on the wasm build.
//
// judge/crash-gate.sh covers the native CLI; this runs the same classes of
// defect — script-triggered panics, unbounded recursion and nesting, the
// silently wrong results, the host limits — against the wasm package in
// Node, where a defect is a trap that leaves the instance unusable (and
// often poisons it for good) instead of an abort. One fresh instance per
// check: that is exactly the state the checks must never reach.
//
// Usage:  cd crates/rtcl-wasm && ./build.sh && cd ../.. && node judge/crash-wasm.mjs
//         node judge/crash-wasm.mjs <name-prefix>
// exit 0 iff every check passes.
import { readFileSync } from 'node:fs';
// The built package (crates/rtcl-wasm/build.sh generates it).
const pkg = new URL('../crates/rtcl-wasm/pkg/', import.meta.url);
const bytes = readFileSync(new URL('./rtcl_wasm_bg.wasm', pkg));
let n = 0, bad = 0;
async function fresh() {
  const mod = await import(new URL('./rtcl_wasm.js?i=' + (++n), pkg));
  await mod.default({ module_or_path: bytes });
  return new mod.RtclHandle();
}
const run = (h, code) => { try { return 'ok:' + JSON.stringify(String(h.exec(code)).slice(0, 70)); } catch (e) { return typeof e === 'string' ? 'tcl:' + e.slice(0, 70) : 'TRAP:' + String(e && e.message).slice(0, 50); } };
async function check(label, code, want) {
  const h = await fresh();
  const r = run(h, code);
  const alive = run(h, 'expr {1+1}');
  const ok = want.test(r) && alive === 'ok:"2"';
  if (!ok) bad++;
  console.log((ok ? 'PASS ' : 'FAIL ') + label.padEnd(46), r.slice(0, 60), alive === 'ok:"2"' ? '' : '| AFTER: ' + alive);
}
async function api(label, fn) {
  const h = await fresh();
  let r; try { r = await fn(h); } catch (e) { r = 'TRAP:' + e; }
  const alive = run(h, 'expr {1+1}');
  const ok = r !== false && alive === 'ok:"2"';
  if (!ok) bad++;
  console.log((ok ? 'PASS ' : 'FAIL ') + label.padEnd(46), String(r).slice(0, 60), alive === 'ok:"2"' ? '' : '| AFTER: ' + alive);
}
const P = 'proc f {n} {';
await check('recursion: plain (100000)',        P + 'if {$n} {f [expr {$n-1}]}}; catch {f 100000} e; set e', /recursion/);
await check('recursion: apply inside catch',     P + 'catch {if {$n} {apply {{n} {f [expr {$n-1}]}} $n}} e}; catch {f 100000} e; set e', /ok/);
await check('recursion: switch/catch',          P + 'switch -- $n {0 {} default {catch {f [expr {$n-1}]}}}}; catch {f 100000} e; set e', /ok/);
await check('recursion: try/finally',           P + 'try {if {$n} {f [expr {$n-1}]}} finally {incr ::x}}; catch {f 100000} e; set e', /recursion/);
await check('recursion: 140 levels completes',   P + 'if {$n} {f [expr {$n-1}]}}; f 140', /ok/);
await check('recursion: namespace unknown loop',  'namespace unknown ::h; catch {nosuch a} e; set e', /recursion|command/);
await check('recursion: alias loop',              'catch {alias a a} e; set e', /would create a loop/);
await check('recursion: nested list 300k',        'set l x; for {set i 0} {$i<300000} {incr i} {set l [list $l]}; string length $l', /ok/);
await check('recursion: nested list drop 200k',   'set l x; for {set i 0} {$i<200000} {incr i} {set l [list $l]}; unset l; expr {7}', /ok:"7/);
await check('recursion: json 100k',               'set s [string repeat {[} 100000]; append s [string repeat {]} 100000]; catch {json decode $s} e; expr {3}', /ok:"3/);
await check('recursion: nested [ 20000',          'set s [string repeat {[list } 20000]1[string repeat {]} 20000]; catch {uplevel #0 $s} e; set e', /nested/);
await check('recursion: expr 100k parens',        'set s [string repeat "(" 100000]; append s 1 [string repeat ")" 100000]; catch {expr $s} e; set e', /nested/);
await check('panic: unicode compare',             'set name 张三; if {$name < "x"} {set r lt} else {set r ge}; set r', /ok:"ge/);
await check('panic: unicode eq',                  'expr {"日本語" == "x"}', /ok:"0/);
await check('panic: format precision',            'string length [format %.65536f 1.5]', /ok:"65538"|tcl/);
await check('panic: string repeat 2^32',          'catch {string repeat x 4294967296} e; set e', /integer value too large/);
await check('panic: lrepeat 2^32',                'catch {lrepeat 4294967296 x} e; set e', /integer value too large/);
await check('silent: + overflows exactly',        '+ 9223372036854775807 1', /ok:"9223372036854775808/);
await check('silent: octal 018 is invalid',       'catch {set v 018; expr {$v + 0}} e; set e', /octal/);
await api('api: set_step_budget trips on while',  h => { h.set_step_budget(200000); return run(h, 'while {1} {}'); });
await api('api: budget 0 lifts it',               h => { h.set_step_budget(200000); h.set_step_budget(0); return run(h, 'set i 0; while {$i < 300000} {incr i}'); });
await api('api: budget covers timerate',          h => { h.set_step_budget(200000); return run(h, 'timerate {} 3600000'); });
await api('api: set_recursion_limit(30)',         h => { h.set_recursion_limit(30); return run(h, P + 'if {$n} {f [expr {$n-1}]}}; catch {f 500} e; set e'); });
await api('callback throwing keeps handle',       h => { h.register_command('boom', () => { throw new Error('host failed'); }); return run(h, 'catch {boom} e; set e'); });
await api('callback re-entrant exec rejected',    h => { let inner; h.set_output_handler(() => { try { h.exec('set ::n 1'); inner = 'allowed'; } catch (e) { inner = 'rejected'; } }); const r = run(h, 'puts x'); return r.startsWith('ok') && inner === 'rejected'; });
console.log(bad === 0 ? 'ALL PASS' : bad + ' FAILED');
process.exit(bad ? 1 : 0);
