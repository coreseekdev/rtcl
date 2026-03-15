# Agent Guidelines for rtcl Project

This document provides guidelines for AI assistants working on the rtcl project.

## Project Overview

rtcl is a lightweight, no-std compatible Tcl interpreter implemented in Rust.

### Key Characteristics
- Minimal implementation focused on embedded systems
- Tcl-compatible syntax and commands
- No external dependencies for core functionality
- Expect-style process automation support

## Code Style Guidelines

### General Principles
1. Keep code simple and focused
2. Avoid over-engineering
3. Prefer standard library over external dependencies
4. Use clear, descriptive names
5. Document complex logic with comments

### Rust Conventions
- Use `clippy` recommendations
- Follow standard Rust formatting (`cargo fmt`)
- Prefer `Result<T, E>` for error handling
- Use `#[cfg(feature)]` for conditional compilation
- Document public APIs with doc comments

### Memory Management
- Prefer stack allocation over heap allocation
- Use `SmallVec` for small collections
- Avoid unnecessary clones
- Consider `Cow` types for shared data

## Architecture

### Crates
- `rtcl-core`: Core interpreter (no-std compatible)
- `rtcl-cli`: Command-line interface
- `rtcl-expect`: Process automation

### Key Types
- `Value`: Tcl value (essentially a string)
- `Interp`: Interpreter state
- `Parser`: Source code parser
- `Error`: Error types

### Command Implementation
Commands are implemented as static methods on `Interp`:
```rust
fn cmd_xxx(interp: &mut Interp, args: &[Value]) -> Result<Value>
```

## Testing

- Unit tests in each crate's `src/` directory
- Integration tests using `.tcl` files
- Run tests: `cargo test`
- Check coverage: `cargo tarpaulin`

## Making Changes

1. Check existing code patterns first
2. Maintain compatibility where possible
3. Add tests for new functionality
4. Update documentation for API changes
5. Run `cargo clippy` before committing

## Common Tasks

### Adding a New Command
1. Add method to `Interp` implementation
2. Register in `register_builtins()`
3. Add tests
4. Update help text in CLI

### Adding a New Expression Function
1. Add to `types/expr.rs`
2. Handle in `eval_expr()`
3. Add tests for edge cases

### Memory Optimization
1. Profile before optimizing
2. Measure impact with benchmarks
3. Consider no-std targets

## References

- Tcl documentation: https://www.tcl.tk/man/
- Jim Tcl (minimal implementation): G:/src.tcl/jimtcl
- Molt (Rust Tcl): G:/src.tcl/molt
