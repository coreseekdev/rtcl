// 开工前置 micro-bench (a) — TASK-JIT-WASM.md §2 / 设计文档 §8 假设 A:
//   wasm→wasm import 调用 vs JS trampoline 调用的单次开销。
//
// 三个被测形态（import 签名 = ABI v2 的 binary_op: (i32,i64,i64)->i64）:
//   direct   : import = 另一个 wasm 模块的导出函数（引擎直连，无 JS 帧）
//   tramp    : import = 普通 JS 函数（BigInt 装箱往返 = 今天 js-sys 路径的形状）
//   typed    : import = WebAssembly.Function 包装的 JS 函数（类型化反射，
//              免参数转换；Node 22 需 --experimental-wasm-type-reflection）
//
// 用法: node bench/scripts/jit_callpath_bench.mjs [iterations]
// 输出: 每形态 ns/call（best-of-5）。

'use strict';

// ── 最小 wasm 二进制组装器（只为这两个固定模块，勿泛化） ──────────────
const enc = s => new TextEncoder().encode(s);
function leb(n) { // unsigned LEB128
  const out = [];
  do { let b = n & 0x7f; n >>>= 7; if (n) b |= 0x80; out.push(b); } while (n);
  return out;
}
function section(id, payload) { return [id, ...leb(payload.length), ...payload.flat(Infinity)]; }
function vec(...items) { return [...leb(items.length), ...items.flat(Infinity)]; }
function name(s) { const b = enc(s); return [...leb(b.length), ...b]; }

// 类型 t0: (i32,i64,i64)->i64  [binary_op]   t1: (i64)->i64  [run]
const TYPES = section(1, vec(
  [0x60, ...vec(0x7f, 0x7e, 0x7e), ...vec(0x7e)],
  [0x60, ...vec(0x7e), ...vec(0x7e)],
));

// callee: (module (func (export "binary_op") (param i32 i64 i64) (result i64)
//          local.get 1  i64.const 1  i64.add  end))
const callee = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
  ...TYPES,
  ...section(3, vec([[0x00]])),                                   // one func, type 0
  ...section(7, vec([[...name('binary_op'), 0x00, 0x00]])),       // export func 0
  ...section(10, (() => {  // one func: no locals; local.get 1; i64.const 1; i64.add; end
    const b = [0x00, 0x20, 0x01, 0x42, 0x01, 0x7c, 0x0b];
    return vec([[...leb(b.length), ...b]]);
  })()),
]);

// caller: (module
//   (import "rtcl" "binary_op" (func $op (param i32 i64 i64) (result i64)))
//   (func (export "run") (param $n i64) (result i64)
//     (local $acc i64)
//     loop $L
//       local.get $n ;; n == 0 → return acc
//       i64.eqz  if  local.get $acc  return  end
//       i32.const 0  local.get $n  i64.const 0  call $op  local.set $acc
//       local.get $n  i64.const 1  i64.sub  local.set $n
//       br $L
//     end
//     unreachable))
function callerModule() {
  const body = [
    0x01, 0x01, 0x7e,        // 1 local group: 1 × i64 ($acc)
    0x03, 0x40,              // loop $L (void)
    0x20, 0x00, 0x50,        // local.get $n; i64.eqz
    0x04, 0x40,              // if (void)
    0x20, 0x01, 0x0f,        //   local.get $acc; return
    0x0b,                    // end if
    0x41, 0x00,              // i32.const 0
    0x20, 0x00,              // local.get $n
    0x42, 0x00,              // i64.const 0
    0x10, 0x00,              // call $op (import = func 0)
    0x21, 0x01,              // local.set $acc
    0x20, 0x00, 0x42, 0x01, 0x7d, 0x21, 0x00, // n -= 1
    0x0c, 0x00,              // br $L
    0x0b,                    // end loop
    0x00,                    // unreachable
    0x0b,                    // end func
  ];
  return new Uint8Array([
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    ...TYPES,
    ...section(2, vec([[...name('rtcl'), ...name('binary_op'), 0x00, 0x00]])),
    ...section(3, vec([[0x01]])),                                // one func, type 1
    ...section(7, vec([[...name('run'), 0x00, 0x01]])),          // export func 1
    ...section(10, vec([[...leb(body.length), ...body]])),
  ]);
}

// ── 计时 ────────────────────────────────────────────────────────────
function bestOf(fn, reps = 5) {
  let best = Infinity;
  for (let i = 0; i < reps; i++) {
    const t0 = process.hrtime.bigint();
    const r = fn();
    const dt = Number(process.hrtime.bigint() - t0);
    if (typeof r === 'bigint' && r < 0n) throw new Error('bad result');
    if (dt < best) best = dt;
  }
  return best;
}

const N = BigInt(process.argv[2] || 100_000_000);
const callerOnly = callerModule();

function runVariant(label, makeImport) {
  const calleeInst = new WebAssembly.Instance(new WebAssembly.Module(callee));
  const inst = new WebAssembly.Instance(new WebAssembly.Module(callerOnly), {
    rtcl: { binary_op: makeImport(calleeInst) },
  });
  inst.exports.run(1n); // warmup + sanity
  const ns = bestOf(() => inst.exports.run(N));
  console.log(`${label.padEnd(8)} ${(ns / Number(N)).toFixed(2).padStart(7)} ns/call   (total ${(ns / 1e6).toFixed(0)} ms for ${N})`);
}

console.log(`iterations = ${N}, engine = node ${process.version} (${typeof WebAssembly.Function === 'function' ? 'type-reflection ON' : 'type-reflection OFF'})`);
runVariant('direct', ci => ci.exports.binary_op);                 // wasm→wasm 直通
runVariant('tramp', () => (op, a, b) => a + 1n);                  // JS trampoline（BigInt 装箱）
if (typeof WebAssembly.Function === 'function') {
  runVariant('typed', ci => new WebAssembly.Function(             // 类型化包装
    { parameters: ['i32', 'i64', 'i64'], results: ['i64'] },
    () => 1n));
}
