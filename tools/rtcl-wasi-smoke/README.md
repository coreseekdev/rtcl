# rtcl-wasi-smoke

rtcl-wasi ABI v0 的冒烟宿主（独立 crate，自带 `[workspace]`，不进 rtcl workspace）。

```sh
# 1) 构建 guest（本仓）
cargo build -p rtcl-wasi --target wasm32-wasip1 --release
# 2) 跑冒烟
cd tools/rtcl-wasi-smoke && cargo build && \
  ./target/debug/rtcl-wasi-smoke ../../target/wasm32-wasip1/release/rtcl_wasi.wasm
```

行为：stub 全部 WASI imports（`fd_write` 真实转发到宿主 stdout）；校验
`rtcl_abi_version` → `rtcl_init` → 跑内置用例（expr / puts / string / 循环 /
错误路径 + 1M 循环 bench），打印 status/结果/耗时。ABI 定义见 rogo 仓
`docs/superpowers/specs/2026-10-18-rtcl-wasi-abi.md`。
