// crash-wasm: the crash-probe classes on the wasm build.
//
// judge/crash-gate.sh covers the native CLI; this runs the same classes of
// defect — script-triggered panics, unbounded recursion and nesting, hangs,
// the silently wrong results, the host limits — against the wasm package in
// Node, where a defect is a trap that leaves the instance unusable (and
// often poisons it for good) instead of an abort. One fresh instance per
// check: that is exactly the state the checks must never reach.
//
// Every check runs in a worker thread of its own, so that a script which
// never returns is a FAIL line here and not a gate that hangs: the thread
// is terminated after HANG_MS.  (On the web the same script is a frozen
// tab; the step budget only sees loops of the bytecode engine, not those
// of a parser or a checker.)
//
// Usage:  cd crates/rtcl-wasm && ./build.sh && cd ../.. && node judge/crash-wasm.mjs
//         node judge/crash-wasm.mjs <label-prefix>
// exit 0 iff every check passes.
import { readFileSync } from 'node:fs';
import { Worker } from 'node:worker_threads';

// The built package (crates/rtcl-wasm/build.sh generates it).
const pkg = new URL('../crates/rtcl-wasm/pkg/', import.meta.url);
const bytes = readFileSync(new URL('./rtcl_wasm_bg.wasm', pkg));
const HANG_MS = 15000;
const prefix = process.argv[2] || '';

// `run` mirrors the native probes' shape: the result or the Tcl error,
// and — asked for afterwards — whether the instance still evaluates.
const WORKER = String.raw`
'use strict';
const { parentPort, workerData: d } = require('node:worker_threads');
const run = (h, code) => { try { return 'ok:' + JSON.stringify(String(h.exec(code)).slice(0, 90)); } catch (e) { return typeof e === 'string' ? 'tcl:' + e.slice(0, 70) : 'TRAP:' + String(e && e.message).slice(0, 50); } };
(async () => {
  let out;
  try {
    const mod = await import(d.pkg + 'rtcl_wasm.js');
    await mod.default({ module_or_path: new Uint8Array(d.bytes) });
    const h = new mod.RtclHandle();
    const s = d.spec;
    let r;
    if (s.budget) h.set_step_budget(s.budget);
    if (s.unbudget) h.set_step_budget(0);
    if (s.recursion) h.set_recursion_limit(s.recursion);
    if (s.kind === 'cb-throw') {
      h.register_command('boom', () => { throw new Error('host failed'); });
      r = run(h, 'catch {boom} e; set e');
    } else if (s.kind === 'child-services') {
      // A child interpreter used to come up on the native clock and
      // console: its puts went nowhere and every after/vwait
      // trapped (wasm32 has no native clock) — the instance was gone.
      let seen = '';
      h.set_output_handler(t => { seen += String(t); });
      const puts = run(h, 'set c [interp create]; $c eval {puts childout}');
      const after = run(h, '$c eval {catch {after 10 {set ::z 1}; vwait ::z} m; set m}');
      const time = run(h, '$c eval {catch {time {expr 1+1}} m; set m}');
      r = (puts.startsWith('ok') && seen === 'childout\n' && !after.startsWith('TRAP') && time.startsWith('ok:')) ? 'ok|' + JSON.stringify(seen) : 'bad|' + puts + '|' + after + '|' + time + '|' + JSON.stringify(seen);
    } else if (s.kind === 'child-budget') {
      // The step budget bounds the whole family: a child created before
      // the budget was armed, or a grandchild, cannot run forever inside
      // interp eval.
      h.set_step_budget(200000);
      const child = run(h, 'set c [interp create]; catch {$c eval {while {1} {}}} m; set m');
      h.set_step_budget(200000);
      const grand = run(h, 'catch {$c eval {set g [interp create]; $g eval {while {1} {}}}} m; set m');
      h.set_step_budget(0);
      const bounded = t => t.startsWith('ok:') && /step budget exceeded/.test(t);
      r = (bounded(child) && bounded(grand)) ? 'ok|bounded' : 'bad|' + child.slice(0, 60) + '|' + grand.slice(0, 60);
    } else if (s.kind === 'cross-instance') {
      // A second instance on the same page registers a callback; the
      // first instance's script names that callback under every slot
      // number the process hands out.  It used to reach it.
      const b = new mod.RtclHandle();
      b.register_command('secret_of_b', x => 'B-SECRET:' + x);
      const own = String(b.exec('secret_of_b x'));
      let leaked = null;
      for (let slot = 0; slot < 8; slot++) {
        try { leaked = String(h.exec('_js_dispatch ' + slot + ' secret_of_b x')); break; } catch (e) { /* not this instance's: expected */ }
      }
      r = (own === 'B-SECRET:x' && leaked === null) ? 'ok|isolated' : 'bad|own=' + own.slice(0, 20) + '|leaked=' + String(leaked).slice(0, 30);
    } else if (s.kind === 'bad-name') {
      // A name that cannot be spelled as one word is refused, not
      // spliced into the wrapper's code.
      let refused = 0;
      for (const name of ['a{b', 'a}b', 'a\\b', '']) {
        try { h.register_command(name, () => 'x'); } catch (e) { refused++; }
      }
      r = 'ok|' + refused;
    } else if (s.kind === 'cb-reenter') {
      let inner;
      h.set_output_handler(() => { try { h.exec('set ::n 1'); inner = 'allowed'; } catch (e) { inner = 'rejected'; } });
      r = run(h, 'puts x').startsWith('ok') && inner === 'rejected' ? 'ok|rejected' : 'bad|' + inner;
    } else {
      r = run(h, s.code);
    }
    out = { r, alive: run(h, 'expr {1+1}') };
  } catch (e) {
    out = { trap: String(e && e.message) };
  }
  parentPort.postMessage(out);
})();
`;

function isolate(spec) {
  return new Promise(resolve => {
    let done = false;
    const finish = m => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      w.terminate().catch(() => {});
      resolve(m);
    };
    const w = new Worker(WORKER, { eval: true, workerData: { pkg: pkg.href, bytes, spec } });
    const timer = setTimeout(() => finish({ hang: true }), HANG_MS);
    w.once('message', finish);
    w.once('error', e => finish({ trap: String(e && e.message) }));
    w.once('exit', () => finish({ trap: 'worker exited without a result' }));
  });
}

let ran = 0, bad = 0;
async function verify(label, spec, want) {
  if (prefix && !label.startsWith(prefix)) return;
  ran++;
  const m = await isolate(spec);
  const ok = !m.hang && !m.trap && want.test(m.r) && m.alive === 'ok:"2"';
  if (!ok) bad++;
  const note = m.hang
    ? 'HANG - no result in ' + HANG_MS + ' ms'
    : m.trap
      ? 'TRAP:' + m.trap.slice(0, 50)
      : m.r.slice(0, 60);
  const after = !m.hang && !m.trap && m.alive !== 'ok:"2"' ? ' | AFTER: ' + m.alive : '';
  console.log((ok ? 'PASS ' : 'FAIL ') + label.padEnd(46), note + after);
}
const check = (label, code, want) => verify(label, { code }, want);

const P = 'proc f {n} {';
// The call-depth limit: tclsh's message and errorCode.
const LIMIT = /too many nested evaluations \(infinite loop\?\)} \{TCL LIMIT STACK/;
await check('recursion: plain (100000)',        P + 'if {$n} {f [expr {$n-1}]}}; catch {f 100000} e; set e', /nested evaluations/);
await check('recursion: apply inside catch',     P + 'catch {if {$n} {apply {{n} {f [expr {$n-1}]}} $n}} e}; catch {f 100000} e; set e', /ok/);
await check('recursion: switch/catch',          P + 'switch -- $n {0 {} default {catch {f [expr {$n-1}]}}}}; catch {f 100000} e; set e', /ok/);
await check('recursion: try/finally',           P + 'try {if {$n} {f [expr {$n-1}]}} finally {incr ::x}}; catch {f 100000} e; set e', /nested evaluations/);
await check('recursion: 140 levels completes',   P + 'if {$n} {f [expr {$n-1}]}}; f 140', /ok/);
await check('recursion: namespace unknown loop',  'namespace unknown ::h; catch {nosuch a} e; set e', /nested evaluations|command/);
await check('recursion: alias loop',              'catch {alias a a} e; set e', /would create a loop/);
await check('recursion: nested list 300k',        'set l x; for {set i 0} {$i<300000} {incr i} {set l [list $l]}; string length $l', /ok/);
await check('recursion: nested list drop 200k',   'set l x; for {set i 0} {$i<200000} {incr i} {set l [list $l]}; unset l; expr {7}', /ok:"7/);
await check('recursion: json 100k',               'set s [string repeat {[} 100000]; append s [string repeat {]} 100000]; catch {json decode $s} e; expr {3}', /ok:"3/);
await check('recursion: nested [ 20000',          'set s [string repeat {[list } 20000]1[string repeat {]} 20000]; catch {uplevel #0 $s} e; set e', /nested/);
await check('recursion: expr 100k parens',        'set s [string repeat "(" 100000]; append s 1 [string repeat ")" 100000]; catch {expr $s} e; set e', /nested/);
await check('recursion: limit message and code',  'proc a {} {a}; catch {a} e; list $e $::errorCode', LIMIT);
await check('panic: unicode compare',             'set name 张三; if {$name < "x"} {set r lt} else {set r ge}; set r', /ok:"ge/);
await check('panic: unicode eq',                  'expr {"日本語" == "x"}', /ok:"0/);
await check('panic: format precision',            'string length [format %.65536f 1.5]', /ok:"65538"|tcl/);
await check('panic: string repeat 2^32',          'catch {string repeat x 4294967296} e; set e', /integer value too large/);
await check('panic: lrepeat 2^32',                'catch {lrepeat 4294967296 x} e; set e', /integer value too large/);
await check('silent: + overflows exactly',        '+ 9223372036854775807 1', /ok:"9223372036854775808/);
// C16（9.1 对齐批 B1）：018 形旧式八进制串的算术错误消息换 9.1 词
// （cannot use non-numeric string "018" as left operand of "+"，oracle
// 实测同词面——rtcl 的 018 仍按 8.6 旧式八进制解析失败（数字字面量
// 9 语法迁移是另一批），故错误仍在、词面随 C16 换新）。
await check('silent: octal 018 is invalid',       'catch {set v 018; expr {$v + 0}} e; set e', /non-numeric string/);
// A `#` in an expression: the checker scanned it as a comment and looped.
await check('hang: # in an expression',           'catch {expr {3 #}} e; set e', new RegExp(String.raw`invalid character \\"#\\"`));
await check('hang: # after an operand',           'catch {expr {1 + #}} e; set e', new RegExp(String.raw`invalid character \\"#\\"`));
// A `tailcall` naming a non-proc: logged by the dispatcher, the words as
// written, the command looked up where the tailcall ran.
await check('silent: tailcall error is logged',   'proc p1 {} { tailcall error x3 }; catch {p1} e o; dict get $o -errorinfo', new RegExp(String.raw`while executing\\n\\"error x3\\"\\n {4}invoked from within\\n\\"p1\\"`));
await check('silent: tailcall words stay values', 'proc w1 {} { tailcall set ::wv {$a [b] "c} }; w1; set ::wv', /ok:"\$a \[b\] \\"c/);
await check('silent: tailcall my in a method',    'oo::class create TC { method go {n acc} { if {$n <= 0} {return $acc} { tailcall my go [expr {$n-1}] [expr {$acc+$n}] } } }; TC create tc; tc go 100 0', /ok:"5050"/);
// `::oo::ObjN::my` is the object's own command: a qualified word reaches it
// from anywhere.
await check('silent: qualified my from a proc',   'oo::class create C { method m {} { return 7 } }; C create c; set n [info object namespace c]; proc pl {n} { ${n}::my m }; pl $n', /ok:"7"/);
await check('silent: unknown-method errorCode',   'oo::class create Q { method m {} {} }; Q create q; catch {q nosuch} e; set ::errorCode', /TCL LOOKUP METHOD nosuch/);
await check('silent: next outside a method',      'catch {::oo::Helpers::next} e; list $e $::errorCode', /next may only be called from inside a method\} \{TCL OO CONTEXT_REQUIRED/);
// `interp alias {} name {} target prefix`: the alias came into being under
// the target's name.
await check('silent: interp alias with a prefix', 'proc t {a b} {list $a $b}; interp alias {} al {} t X; al Y', /ok:"X Y"/);

// `interp hide`: gone from every table an invocation resolves through —
// a body compiled before the hide included — until exposed again.
await check('interp: a hidden command is not callable', 'interp create c; interp eval c {proc f {} {set x 1}}; interp hide c set; list [interp eval c {catch {f} e}] [interp invokehidden c set e] [interp hidden c] [interp eval c {info commands set}]', /ok:"1 \{invalid command name ..set..\} set ./);
await check('interp: expose brings it back', 'interp create c; interp hide c set; interp expose c set myset; list [interp hidden c] [interp eval c {myset a 1}]', /ok:"\{\} 1"/);
// A `-safe` interpreter: the commands that reach outside start hidden,
// and its own `interp` lets none of them back.
await check('interp: -safe hides and still computes', 'interp create s -safe; list [interp issafe s] [interp eval s {catch {interp hide {} clock} m o; dict get $o -errorcode}] [interp eval s {catch {interp invokehidden {} clock seconds} m o; dict get $o -errorcode}] [interp eval s {interp create k; interp issafe k}] [interp eval s {info exists env}] [interp eval s {expr 1 + 1}]', /ok:"1 \{TCL OPERATION INTERP UNSAFE\} \{TCL OPERATION INTERP UNSAFE\} 1 0 2"/);
await check('interp: -safe cannot expose', 'interp create s -safe; interp eval s {interp create k; catch {interp expose k exec} m o; dict get $o -errorcode}', /TCL OPERATION INTERP UNSAFE/);
// Aliases run their words in another interpreter; a loop of them is
// refused as it is made, not run out.
await check('interp: alias loop refused', 'interp create l4; interp alias l4 x {} y; catch {interp alias {} y l4 x} m o; dict get $o -errorcode', /TCL OPERATION INTERP ALIASLOOP/);
await check('interp: self-target loop refused', 'catch {interp alias {} z {} z} m o; dict get $o -errorcode', /TCL OPERATION INTERP ALIASLOOP/);
await check('interp: aliases between any two', 'interp create a; interp create b; interp alias a up {} set ::v; interp alias a side b set y; interp alias {} down a set q; list [interp eval a {up 7}] [interp eval a {side 5}] [down 3] [set ::v] [interp eval b {set y}] [interp eval a {set q}]', /ok:"7 5 3 7 5 3"/);
await check('interp: alias into a grandchild', 'interp create s; interp eval s {interp create g}; interp alias {s g} ga {} set ::w 9; list [interp eval {s g} {ga}] [set ::w]', /ok:"9 9"/);
await check('interp: target gone, alias gone', 'interp create a; interp create b; interp alias a t b set y 1; interp delete b; list [interp aliases a] [interp eval a {catch {t} m o; dict get $o -errorcode}]', /ok:"\{\} \{TCL LOOKUP COMMAND t\}"/);
await check('interp: target error is the caller', 'interp create a; interp alias a e {} error deep; interp eval a {catch {e} m o; dict get $o -errorinfo}', /ok:"deep.n    invoked from within.n..e.."/);
await check('interp: no delete while evaluated', 'interp create d; interp alias d w {} interp delete d; interp eval d w', /cannot delete interpreter/);
await check('interp: renamed interpreter command', 'interp create s; rename s s2; list [s2 eval {expr 1 + 1}] [interp exists s] [interp delete s] [interp exists s]', /ok:"2 1 \{\} 0"/);
await check('interp: eval of a missing one', 'catch {interp eval nope x} m o; dict get $o -errorcode', /TCL LOOKUP INTERP nope/);

// A child interpreter starts from its parent: the host's console and
// clock, and the limits (a counter of its own ran forever in a budget).
await verify('child: host services inherited, no trap', { kind: 'child-services' }, /ok\|"childout\\n"/);
await verify('child: step budget covers the family', { kind: 'child-budget' }, /ok\|bounded/);

// The dispatch bridge's slot number is not a capability: an instance
// reaches its own callbacks and no others.
await verify('isolation: _js_dispatch stays in its instance', { kind: 'cross-instance' }, /ok\|isolated/);
await verify('isolation: unspellable command names refused', { kind: 'bad-name' }, /ok\|4/);

await verify('api: set_step_budget trips on while', { budget: 200000, code: 'while {1} {}' }, /tcl/);
await verify('api: budget 0 lifts it',              { budget: 200000, unbudget: true, code: 'set i 0; while {$i < 300000} {incr i}' }, /ok/);
await verify('api: budget covers timerate',         { budget: 200000, code: 'timerate {} 3600000' }, /tcl/);
await verify('api: set_recursion_limit(30)',        { recursion: 30, code: P + 'if {$n} {f [expr {$n-1}]}}; catch {f 500} e; set e' }, /nested evaluations/);
await verify('api: callback throwing keeps handle', { kind: 'cb-throw' }, /host failed/);
await verify('callback re-entrant exec rejected',   { kind: 'cb-reenter' }, /ok\|rejected/);
console.log(bad === 0 ? `ALL PASS (${ran})` : `${bad} FAILED of ${ran}`);
process.exit(bad ? 1 : 0);
