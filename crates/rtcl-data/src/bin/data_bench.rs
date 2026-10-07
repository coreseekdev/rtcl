//! rtcl-data 微基准（不经 interp，测库函数吞吐）。
//!
//! 跑法：`cargo run -p rtcl-data --bin data_bench --release`
//! 记录：结果按现有风格追加到 bench/BASELINE.md 的 rtcl-data 节。

use std::time::Instant;

fn bench<F: FnMut()>(name: &str, iters: u32, mut f: F) {
    // 预热一次，避免首轮分配/页错误混入
    f();
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    let per = t.elapsed().as_nanos() / iters as u128;
    println!("{name:<24} {per} ns/op");
}

fn main() {
    let yaml_doc = std::fs::read_to_string("bench/fixture.yaml")
        .unwrap_or_else(|_| "a: 1\nb: [x, y, z]\nc: {d: e}\n".repeat(128));
    let u = "https://WWW.Example.com/A/B/?utm_source=x&id=2#frag";
    bench("yaml_decode_4kb", 2000, || {
        let _: serde_yaml::Value = serde_yaml::from_str(&yaml_doc).unwrap();
    });
    bench("url_normalize", 100_000, || {
        let _ = rtcl_data::url::normalize_url(u).unwrap();
    });
    bench("sha256_64b", 100_000, || {
        let _ = rtcl_data::sha::sha256_hex("The quick brown fox jumps over the lazy dog");
    });
}
