# Claude Code Guidelines for rtcl Project

This file contains project-specific instructions for Claude Code when working on the rtcl project.

## Project Context

rtcl is a lightweight Tcl-compatible scripting language implemented in Rust, designed for embedded systems and no-std environments.

## Build Commands

```bash
# Build all crates
cargo build

# Build in release mode
cargo build --release

# Run tests
cargo test

# Check for linting issues
cargo clippy

# Format code
cargo fmt
```

## Project Structure

```
rtcl/
├── Cargo.toml          # Workspace configuration
├── crates/
│   ├── rtcl-core/    # Core interpreter library
│   ├── rtcl-cli/     # CLI application
│   └── rtcl-expect/  # Process automation
├── tests/            # Integration tests (optional)
├── README.md          # Project documentation
├── Agent.md           # Agent guidelines
└── Claude.md          # This file
```

## Key Implementation Details

### Value Type
- Everything is a string in Tcl
- Internal representation is cached for optimization
- Uses `SmallVec` for small strings

### Parser
- Recursive descent parser
- Handles braces `{}`, quotes `""`, and command substitution `[]`
- Variable references `$var` and `${var}`

### Interpreter
- Command dispatch table
- Variable storage (global scope only currently)
- Recursion limit (1000 by default)

## Adding New Features

1. Check existing implementations in jimtcl and molt for reference
2. Keep implementations minimal
3. Ensure no-std compatibility where possible
4. Add tests for new functionality

5. Update documentation

## Testing Strategy

- Unit tests for individual functions
- Integration tests using Tcl scripts
- Manual testing with CLI

## Code Review Checklist

- [ ] No unnecessary dependencies added
- [ ] Code compiles without warnings
- [ ] Tests pass
- [ ] Documentation updated
- [ ] No-std compatibility maintained (for rtcl-core)
- [ ] Clippy passes

## Common Patterns

### Adding a Built-in Command
```rust
// In interp.rs
fn cmd_newcommand(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Implementation
}

// In register_builtins()
self.register_builtin("newcommand", Self::cmd_newcommand);
```

### Adding Expression Function
```rust
// In types/expr.rs
pub fn eval_newfunc(interp: &mut Interp, args: &[Value]) -> Result<Value> {
    // Implementation
}
```
