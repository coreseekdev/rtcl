//! The web runtime's stack: 8 MiB, as nsh's own (`.cargo/config.toml`).

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg=-zstack-size=8388608");
    }
}
