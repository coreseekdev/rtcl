# rtcl - A Lightweight Tcl-Compatible Scripting Language

A minimal, no-std compatible Tcl interpreter implemented in Rust.

## Features

- **Lightweight**: Designed for embedded systems and resource-constrained environments
- **No-std Compatible**: Core library works in `no_std` environments with `alloc`
- **Tcl Compatible**: Supports familiar Tcl syntax and commands
- **Expect Support**: Process automation capabilities similar to classic `expect`

## Quick Start

### Build

```bash
cargo build --release
```

### Run a Script

```bash
# Execute a script file
./target/release/rtcl -f script.tcl

# Evaluate a command
./target/release/rtcl -c "puts hello"

# Interactive REPL
./target/release/rtcl -i
```

### REPL Commands

```
rtcl> set x 42
42
rtcl> puts "x = $x"
x = 42
rtcl> expr 1 + 2 * 3
7
rtcl> .exit
```

## Supported Commands

### Core Commands

| Command | Description |
|---------|-------------|
| `set` | Get or set a variable |
| `puts` | Print a string |
| `if` | Conditional execution |
| `while` | While loop |
| `for` | For loop |
| `foreach` | Iterate over list |
| `expr` | Evaluate expression |
| `proc` | Define procedure (limited) |
| `return` | Return from procedure |
| `break` | Exit loop |
| `continue` | Continue to next iteration |

### List Commands

| Command | Description |
|---------|-------------|
| `list` | Create a list |
| `llength` | Get list length |
| `lindex` | Get list element |
| `lappend` | Append to list variable |
| `concat` | Concatenate strings |

### String Commands

| Command | Description |
|---------|-------------|
| `string length` | String length |
| `string range` | Substring |
| `string tolower` | Lowercase |
| `string toupper` | Uppercase |
| `string trim` | Trim whitespace |

### Other Commands

| Command | Description |
|---------|-------------|
| `incr` | Increment variable |
| `append` | Append to string variable |
| `catch` | Catch errors |
| `error` | Throw error |
| `info exists` | Check variable |
| `info commands` | List commands |
| `rename` | Rename/delete command |
| `eval` | Evaluate script |
| `uplevel` | Evaluate in caller scope |

## Expression Syntax

Supports standard Tcl expressions:

```
expr 1 + 2 * 3       ; => 7
expr $x > 0          ; => 1 (true)
expr {$a == $b}      ; comparison
expr {abs(-5)}       ; function call
```

Supported operators: `+`, `-`, `*`, `/`, `%`, `<`, `>`, `<=`, `>=`, `==`, `!=`, `&&`, `||`, `!`

Supported functions: `abs`, `int`, `double`, `round`, `floor`, `ceil`, `sqrt`, `pow`, `sin`, `cos`, `tan`, `log`, `exp`, `min`, `max`

## Expect Module

```rust
use rtcl_expect::{spawn, ExpectError};

let mut proc = spawn("ssh", &["user@host"])?;
proc.expect("password:", Duration::from_secs(10))?;
proc.send_line("mypassword")?;
```

## Project Structure

```
rtcl/
├── crates/
│   ├── rtcl-core/      # Core interpreter (no-std compatible)
│   ├── rtcl-cli/       # Command-line interface
│   └── rtcl-expect/    # Expect-style process automation
└── tasks/             # Task files for development
```

## No-std Usage

```rust
#![no_std]

extern crate alloc;

use rtcl_core::{Interp, Value};

let mut interp = Interp::new();
interp.eval("set x 42").unwrap();
```

## Differences from Standard Tcl

1. Procedure definitions are simplified
2. No namespace support yet
3. Limited error stack traces
4. Some string commands may differ slightly

## License

MIT OR Apache-2.0
