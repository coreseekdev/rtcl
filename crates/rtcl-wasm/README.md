# rtcl-wasm

rtcl Tcl interpreter compiled to WASM for use in web pages. No backend, no
node toolchain required — the build output is a plain ES module plus a
`.wasm` file.

## Build

```bash
./build.sh            # release build + wasm-bindgen -> pkg/
./build.sh --dev      # debug build
node test.js          # run node test suite against pkg/
```

Requires: `rustup target add wasm32-unknown-unknown`, and `wasm-bindgen-cli`
matching the `wasm-bindgen` version in `Cargo.lock` (schema versions must
match exactly).

## Try it in a browser

```bash
./build.sh
python3 -m http.server 8000        # from this directory
# open http://localhost:8000/www/index.html
```

The page is a terminal-style REPL: line editing, command history (↑/↓),
multi-line continuation for unbalanced braces/quotes, `puts` output routed
into the page, and errors shown in red.

## JS API

```js
import init, { RtclHandle } from './pkg/rtcl_wasm.js';
await init();

const rtcl = new RtclHandle();

// Execute a script; returns the result as a string, throws on Tcl error.
rtcl.exec('set x 42; expr {$x * 2}');        // -> "84"

// Register a JS function as a Tcl command (per-interpreter).
rtcl.register_command('add', (a, b) => Number(a) + Number(b));
rtcl.exec('add 20 22');                      // -> "42"

// Remove it again (deletes the Tcl proc wrapper too).
rtcl.unregister_command('add');              // -> true

// Route puts/stdout to a JS callback.
rtcl.set_output_handler((s) => console.log('[tcl]', s));
rtcl.exec('puts "hello"');                   // logs "hello"

// Resource limits. Unbounded by default is Tcl semantics: `while {1} {}`
// spins for as long as the page lives. A script from elsewhere gets a
// budget — commands, loop iterations and bracket evaluations all count;
// waiting (vwait, update, timers) does not:
rtcl.set_step_budget(5_000_000);             // "eval step budget exceeded" past it
rtcl.set_step_budget(0);                     // lift it again

// Recursion and script re-entry depth. The default (150) is what fits
// the ~1MiB stack the JS engine gives wasm; a deeper limit overflows
// that stack instead of raising the Tcl error, and that leaves the
// instance unusable:
rtcl.set_recursion_limit(150);
```

Notes:

- Args reach the JS function as strings; the return value is converted with
  JS `String()` semantics (numbers, objects, null all work).
- A JS exception becomes a Tcl error:
  `JS command 'name' error: <message>`.
- Command registrations are scoped per `RtclHandle` — two interpreters never
  see each other's commands.
- `register_command` defines a real Tcl `proc` wrapper, so `rename`,
  `info commands`, etc. behave naturally.
- A trap inside wasm (a stack overflow past the recursion limit, a bug)
  is not an error the interpreter can raise: it throws a `RuntimeError` /
  `RangeError` from `exec` and the handle cannot be used afterwards. Run
  untrusted scripts in a Worker you can terminate, and set the limits
  above. `judge/crash-wasm.mjs` pins the defects of this kind that were
  found in the 2026-11-23 audit.
