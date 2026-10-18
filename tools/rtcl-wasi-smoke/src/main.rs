//! rtcl-wasi ABI v0 冒烟宿主（独立工具，非 rtcl 仓成员）。
//! 用法：rtcl-wasi-smoke <rtcl_wasi.wasm>
//! 行为：stub 全部 WASI imports（返回 0），实例化后跑内置用例集，
//! 打印每个用例的 status/结果/耗时。验证 ABI 载入→eval→取回文本全链路。

use std::time::Instant;
use wasmtime::{bail, Caller, Engine, ExternType, Instance, Linker, Memory, Module, Result, Store, TypedFunc, Val, ValType};

struct Guest {
    store: Store<()>,
    mem: Memory,
    abi_version: TypedFunc<(), u32>,
    alloc: TypedFunc<u32, u32>,
    free: TypedFunc<(u32, u32), ()>,
    init: TypedFunc<u32, u32>,
    eval: TypedFunc<(u32, u32, u32), i32>,
    result_len: TypedFunc<u32, u32>,
    result_copy: TypedFunc<(u32, u32, u32), u32>,
    error_len: TypedFunc<u32, u32>,
    error_copy: TypedFunc<(u32, u32, u32), u32>,
}

impl Guest {
    fn load(wasm: &str) -> Result<Guest> {
        let engine = Engine::default();
        let module = Module::from_file(&engine, wasm)?;
        let mut linker: Linker<()> = Linker::new(&engine);
        // stub 所有 imports（wasm32-wasip1 std 面）：返回 0 = 成功、时间/随机为 0。
        for import in module.imports() {
            match import.ty() {
                ExternType::Func(ft) => {
                    let result_types: Vec<ValType> = ft.results().collect();
                    let ft = ft.clone();
                    if import.module() == "wasi_snapshot_preview1" && import.name() == "fd_write" {
                        // fd_write 真实转发到宿主 stdout/stderr（puts 验收用）。
                        linker.func_new(
                            import.module(),
                            import.name(),
                            ft,
                            |mut caller: Caller<'_, ()>, params: &[Val], results: &mut [Val]| {
                                let mem = caller
                                    .get_export("memory")
                                    .and_then(|e| e.into_memory())
                                    .ok_or_else(|| wasmtime::Error::msg("no memory"))?;
                                let fd = params[0].unwrap_i32();
                                let iovs = params[1].unwrap_i32() as u32 as usize;
                                let n = params[2].unwrap_i32() as u32 as usize;
                                let out_ptr = params[3].unwrap_i32() as u32 as usize;
                                let mut buf = Vec::new();
                                {
                                    let data = mem.data(&caller);
                                    for i in 0..n {
                                        let base = iovs + i * 8;
                                        let ptr = u32::from_le_bytes(data[base..base + 4].try_into().unwrap()) as usize;
                                        let len = u32::from_le_bytes(data[base + 4..base + 8].try_into().unwrap()) as usize;
                                        buf.extend_from_slice(&data[ptr..ptr + len]);
                                    }
                                }
                                if fd == 1 || fd == 2 {
                                    use std::io::Write;
                                    std::io::stdout().write_all(&buf).ok();
                                }
                                let total = buf.len() as u32;
                                let data = mem.data_mut(&mut caller);
                                data[out_ptr..out_ptr + 4].copy_from_slice(&total.to_le_bytes());
                                results[0] = Val::I32(0);
                                Ok(())
                            },
                        )?;
                        continue;
                    }
                    linker.func_new(
                        import.module(),
                        import.name(),
                        ft,
                        move |_caller: Caller<'_, ()>, _params: &[Val], results: &mut [Val]| {
                            for (slot, ty) in results.iter_mut().zip(result_types.iter()) {
                                *slot = match ty {
                                    ValType::I32 => Val::I32(0),
                                    ValType::I64 => Val::I64(0),
                                    ValType::F32 => Val::F32(0),
                                    ValType::F64 => Val::F64(0),
                                    other => bail!("unsupported stub result type: {other:?}"),
                                };
                            }
                            Ok(())
                        },
                    )?;
                }
                other => bail!("unsupported import {}.{} kind {other:?}", import.module(), import.name()),
            }
        }
        let mut store = Store::new(&engine, ());
        let instance: Instance = linker.instantiate(&mut store, &module)?;
        let mem = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| wasmtime::Error::msg("memory export missing"))?;
        macro_rules! tf {
            ($n:literal, $p:ty, $r:ty) => {
                instance
                    .get_typed_func::<$p, $r>(&mut store, $n)
                    .map_err(|e| e.context(format!("export {}", $n)))?
            };
        }
        Ok(Guest {
            abi_version: tf!("rtcl_abi_version", (), u32),
            alloc: tf!("rtcl_alloc", u32, u32),
            free: tf!("rtcl_free", (u32, u32), ()),
            init: tf!("rtcl_init", u32, u32),
            eval: tf!("rtcl_eval", (u32, u32, u32), i32),
            result_len: tf!("rtcl_result_len", u32, u32),
            result_copy: tf!("rtcl_result_copy", (u32, u32, u32), u32),
            error_len: tf!("rtcl_error_len", u32, u32),
            error_copy: tf!("rtcl_error_copy", (u32, u32, u32), u32),
            mem,
            store,
        })
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<u32> {
        let ptr = self.alloc.call(&mut self.store, bytes.len() as u32)?;
        let data = self.mem.data_mut(&mut self.store);
        data[ptr as usize..ptr as usize + bytes.len()].copy_from_slice(bytes);
        Ok(ptr)
    }

    fn read_string(&self, ptr: u32, len: u32) -> String {
        let data = self.mem.data(&self.store);
        String::from_utf8_lossy(&data[ptr as usize..(ptr + len) as usize]).to_string()
    }

    fn eval(&mut self, handle: u32, script: &str) -> Result<(i32, String)> {
        let ptr = self.write_bytes(script.as_bytes())?;
        let status = self.eval.call(&mut self.store, (handle, ptr, script.len() as u32))?;
        let (len_fn, copy_fn) = if status == 0 {
            (self.result_len.clone(), self.result_copy.clone())
        } else {
            (self.error_len.clone(), self.error_copy.clone())
        };
        let len = len_fn.call(&mut self.store, handle)?;
        let out = self.alloc.call(&mut self.store, len.max(1))?;
        let n = copy_fn.call(&mut self.store, (handle, out, len))?;
        let text = self.read_string(out, n);
        self.free.call(&mut self.store, (out, len.max(1)))?;
        self.free.call(&mut self.store, (ptr, script.len() as u32))?;
        Ok((status, text))
    }
}

fn main() -> Result<()> {
    let wasm = std::env::args()
        .nth(1)
        .ok_or_else(|| wasmtime::Error::msg("usage: rtcl-wasi-smoke <rtcl_wasi.wasm>"))?;
    let mut g = Guest::load(&wasm)?;

    let abi = g.abi_version.call(&mut g.store, ())?;
    println!("ABI: {abi} (expect 1)");
    let handle = g.init.call(&mut g.store, abi)?;
    if handle == 0 {
        bail!("rtcl_init returned 0");
    }
    println!("HANDLE: {handle}");

    let cases: &[(&str, &str, i32)] = &[
        ("expr", "expr {1+1}", 0),
        ("puts", "puts hello-rtcl-wasi", 0),
        ("string", "string map {a A} banana", 0),
        ("loop100k", "set acc 0; for {set i 0} {$i < 100000} {incr i} {incr acc $i}; set acc", 0),
        ("error", "error boom", 1),
    ];
    let mut failures = 0u32;
    for (name, script, expect) in cases {
        let t0 = Instant::now();
        let (status, text) = g.eval(handle, script)?;
        let dt = t0.elapsed();
        let ok = status == *expect;
        if !ok {
            failures += 1;
        }
        println!(
            "[{}] {name}: status={status} (expect {expect}) {:?} [{dt:?}]",
            if ok { "PASS" } else { "FAIL" },
            text
        );
    }
    let t0 = Instant::now();
    let (status, text) = g.eval(handle, "set acc 0; for {set i 0} {$i < 1000000} {incr i} {incr acc $i}; set acc")?;
    println!("[bench] 1M-loop: status={status} result={text} [{:?}]", t0.elapsed());
    println!("{}", if failures == 0 { "RESULT: ALL PASS" } else { "RESULT: FAILURES" });
    std::process::exit(if failures == 0 { 0 } else { 1 });
}
