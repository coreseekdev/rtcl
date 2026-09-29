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
