//! C99 数学函数后端（tcl::mathfunc:: 的 9.1 全集，TIP 745/TIP 521）。
//!
//! # 为什么 unix 目标直连系统 libm（extern "C"）
//!
//! tclsh 9.1 的 BuiltinFuncTable（generic/tclBasic.c）把 acosh/asinh/atanh/
//! cbrt/fdim/erf/erfc/exp2/expm1/tgamma/lgamma/log1p/log2/logb/nextafter/
//! remainder/trunc/copysign/fma/ldexp/frexp/modf/remquo 全部直映射到 C 库
//! 同名函数；oracle 与 rtcl 同机同 glibc，直连同一 `.so` 才能逐字节一致。
//! Rust std 的对应 f64 方法走的是 rust-libm（musl 移植），与 glibc 在最后
//! 一个 ulp 上**实测有分歧**（本机 pin 形：atanh(0.5)、cbrt(2)、
//! expm1(1) 都差 1 ulp），因此不能用作 unix 后端。
//!
//! 非 unix 目标（wasm32 等）退回 Rust std / 手写实现：wasm 门禁只要求
//! 编译通过与语义近似，不做逐字节钉形。
//!
//! # errno 口径（Tcl_CheckDoubleResult，tclBasic.c）
//!
//! tclsh 清 errno 后调用 C 函数，然后：
//! - 结果 NaN（或 errno EDOM）→ `domain error: argument not in valid range`；
//! - errno ERANGE 且结果为 0.0 或 ±Inf → **接受**（溢出给 Inf、下溢给 0）；
//! - 其余 errno ≠ 0（glibc 的 subnormal 下溢即 ERANGE）→ 报错，文案按
//!   TclExprFloatError 取 `floating-point value too large to represent`。
//!
//! Rust 不经 errno，所以等价判定用结果分类实现：NaN→域错误；对 **实测
//! 会置 ERANGE** 的子正规结果函数（atanh/asinh/nextafter/erfc，本机
//! oracle 逐个探针确认）报 too-large；其余函数（erf/exp2/exp/log1p/cbrt/
//! fma/remainder/ldexp——同样逐个探针确认）子正规结果原样返回。

/// unix 后端：直连系统 libm（与 tclsh 同一实现，逐字节一致的前提）。
/// 每个 extern 都包一层安全函数——unsafe extern fn 指针不实现 Fn，
/// 包完才能与 wasm 后端共用同一分派面。
#[cfg(unix)]
pub(crate) mod sys {
    /// 真实 libm 符号名集中在一个子模块里，避免与上面的安全包装同名
    /// （E0428）；`lgamma` 等 POSIX 符号在 glibc 里带版本别名，直接用
    /// 默认符号名链接即可。
    #[allow(non_snake_case)]
    mod ffi {
        extern "C" {
            pub fn acosh(x: f64) -> f64;
            pub fn asinh(x: f64) -> f64;
            pub fn atanh(x: f64) -> f64;
            pub fn cbrt(x: f64) -> f64;
            pub fn fdim(x: f64, y: f64) -> f64;
            pub fn erf(x: f64) -> f64;
            pub fn erfc(x: f64) -> f64;
            pub fn exp2(x: f64) -> f64;
            pub fn expm1(x: f64) -> f64;
            pub fn tgamma(x: f64) -> f64;
            pub fn lgamma(x: f64) -> f64;
            pub fn log1p(x: f64) -> f64;
            pub fn log2(x: f64) -> f64;
            pub fn logb(x: f64) -> f64;
            pub fn nextafter(x: f64, y: f64) -> f64;
            pub fn remainder(x: f64, y: f64) -> f64;
            pub fn trunc(x: f64) -> f64;
            pub fn copysign(x: f64, y: f64) -> f64;
            pub fn fma(x: f64, y: f64, z: f64) -> f64;
            pub fn frexp(x: f64, exp: *mut i32) -> f64;
            pub fn ldexp(x: f64, exp: i32) -> f64;
            pub fn modf(x: f64, iptr: *mut f64) -> f64;
            pub fn remquo(x: f64, y: f64, quo: *mut i32) -> f64;
        }
    }

    pub(crate) fn acosh(x: f64) -> f64 {
        unsafe { ffi::acosh(x) }
    }
    pub(crate) fn asinh(x: f64) -> f64 {
        unsafe { ffi::asinh(x) }
    }
    pub(crate) fn atanh(x: f64) -> f64 {
        unsafe { ffi::atanh(x) }
    }
    /// cbrt 特殊：compiler-builtins（rust-libm）在静态对象里抢出了
    /// `cbrt`/`copysign`/`fdim`/`fma`/`trunc` 五个符号（nm 实测），
    /// `extern "C"` 声明会被绑到 rust-libm 的实现——cbrt 两家差 1 ulp
    /// （cbrt(2)：rust …732 vs glibc …734，tclsh 用 glibc）。经
    /// dlopen 既有 libm.so.6 + dlsym 取动态定义绕开静态劫持；dlopen
    /// 失败（病态环境）才退回 rust 实现。其余四个是 IEEE 精确运算，
    /// 两实现逐位相同，不受劫持影响。
    pub(crate) fn cbrt(x: f64) -> f64 {
        type CbrtFn = unsafe extern "C" fn(f64) -> f64;
        unsafe extern "C" {
            fn dlopen(filename: *const u8, flags: i32) -> *mut core::ffi::c_void;
            fn dlsym(handle: *mut core::ffi::c_void, symbol: *const u8) -> *mut core::ffi::c_void;
        }
        static CBRT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        let addr = *CBRT.get_or_init(|| unsafe {
            let handle = dlopen(b"libm.so.6\0".as_ptr(), 1 /* RTLD_LAZY */);
            if handle.is_null() {
                return 0;
            }
            dlsym(handle, b"cbrt\0".as_ptr()) as usize
        });
        if addr != 0 {
            let f: CbrtFn = unsafe { std::mem::transmute(addr) };
            return unsafe { f(x) };
        }
        x.cbrt()
    }
    pub(crate) fn fdim(x: f64, y: f64) -> f64 {
        unsafe { ffi::fdim(x, y) }
    }
    pub(crate) fn erf(x: f64) -> f64 {
        unsafe { ffi::erf(x) }
    }
    pub(crate) fn erfc(x: f64) -> f64 {
        unsafe { ffi::erfc(x) }
    }
    pub(crate) fn exp2(x: f64) -> f64 {
        unsafe { ffi::exp2(x) }
    }
    pub(crate) fn expm1(x: f64) -> f64 {
        unsafe { ffi::expm1(x) }
    }
    pub(crate) fn tgamma(x: f64) -> f64 {
        unsafe { ffi::tgamma(x) }
    }
    pub(crate) fn lgamma(x: f64) -> f64 {
        unsafe { ffi::lgamma(x) }
    }
    pub(crate) fn log1p(x: f64) -> f64 {
        unsafe { ffi::log1p(x) }
    }
    pub(crate) fn log2(x: f64) -> f64 {
        unsafe { ffi::log2(x) }
    }
    pub(crate) fn logb(x: f64) -> f64 {
        unsafe { ffi::logb(x) }
    }
    pub(crate) fn nextafter(x: f64, y: f64) -> f64 {
        unsafe { ffi::nextafter(x, y) }
    }
    pub(crate) fn remainder(x: f64, y: f64) -> f64 {
        unsafe { ffi::remainder(x, y) }
    }
    pub(crate) fn trunc(x: f64) -> f64 {
        unsafe { ffi::trunc(x) }
    }
    pub(crate) fn copysign(x: f64, y: f64) -> f64 {
        unsafe { ffi::copysign(x, y) }
    }
    pub(crate) fn fma(x: f64, y: f64, z: f64) -> f64 {
        unsafe { ffi::fma(x, y, z) }
    }
    pub(crate) fn frexp(x: f64) -> (f64, i32) {
        let mut e: i32 = 0;
        let f = unsafe { ffi::frexp(x, &mut e) };
        (f, e)
    }
    pub(crate) fn modf(x: f64) -> (f64, f64) {
        let mut i: f64 = 0.0;
        let f = unsafe { ffi::modf(x, &mut i) };
        (i, f)
    }
    pub(crate) fn remquo(x: f64, y: f64) -> (f64, i32) {
        let mut q: i32 = 0;
        let r = unsafe { ffi::remquo(x, y, &mut q) };
        (r, q)
    }
    pub(crate) fn ldexp(x: f64, exp: i32) -> f64 {
        unsafe { ffi::ldexp(x, exp) }
    }
}

/// 非 unix 后端：Rust std 有稳定实现的走 std，其余手写（wasm 编译面）。
#[cfg(not(unix))]
pub(crate) mod sys {
    pub(crate) fn acosh(x: f64) -> f64 {
        x.acosh()
    }
    pub(crate) fn asinh(x: f64) -> f64 {
        x.asinh()
    }
    pub(crate) fn atanh(x: f64) -> f64 {
        x.atanh()
    }
    pub(crate) fn cbrt(x: f64) -> f64 {
        x.cbrt()
    }
    pub(crate) fn fdim(x: f64, y: f64) -> f64 {
        if x > y {
            x - y
        } else {
            0.0
        }
    }
    pub(crate) fn erf(x: f64) -> f64 {
        erfc(x)
    }
    /// Abramowitz–Stegun 7.1.26 的互补误差函数近似（仅 wasm 编译面；
    /// 非逐字节钉形目标）。
    pub(crate) fn erfc(x: f64) -> f64 {
        let a1: f64 = 0.254829592;
        let a2: f64 = -0.284496736;
        let a3: f64 = 1.421413741;
        let a4: f64 = -1.453152027;
        let a5: f64 = 1.061405429;
        let p: f64 = 0.3275911;
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        let x = x.abs();
        let t = 1.0 / (1.0 + p * x);
        let y = 1.0
            - ((((t * a5 + a4) * t + a3) * t + a2) * t + a1) * t * (-x * x).exp();
        sign * y
    }
    pub(crate) fn exp2(x: f64) -> f64 {
        x.exp2()
    }
    pub(crate) fn expm1(x: f64) -> f64 {
        x.exp_m1()
    }
    /// Lanczos 近似（g=7, n=9；仅 wasm 编译面）。极点（非正整数）给 NaN。
    pub(crate) fn tgamma(x: f64) -> f64 {
        if x < 0.5 {
            // 反射公式：Γ(x) = π / (sin(πx) · Γ(1-x))
            let s = (core::f64::consts::PI * x).sin();
            if s == 0.0 {
                return f64::NAN;
            }
            core::f64::consts::PI / (s * tgamma(1.0 - x))
        } else {
            const G: f64 = 7.0;
            const C: [f64; 9] = [
                0.99999999999980993,
                676.5203681218851,
                -1259.1392167224028,
                771.32342877765313,
                -176.61502916214059,
                12.507343278686905,
                -0.13857109526572012,
                9.9843695780195716e-6,
                1.5056327351493116e-7,
            ];
            let z = x - 1.0;
            let mut a = C[0];
            let t = z + G - 0.5;
            for (i, c) in C.iter().enumerate().skip(1) {
                a += c / (z + i as f64);
            }
            let w = t / (2.0 * core::f64::consts::PI);
            (0.5 * core::f64::consts::PI + w.sin()) * a * t.powf(z + 0.5) * (-t).exp()
        }
    }
    pub(crate) fn lgamma(x: f64) -> f64 {
        if x == 0.0 || x.is_infinite() {
            return if x.is_sign_negative() || x == 0.0 {
                f64::INFINITY
            } else {
                f64::INFINITY
            };
        }
        if x < 0.0 && x.fract() == 0.0 {
            return f64::INFINITY; // 极点
        }
        (tgamma(x).abs()).ln()
    }
    pub(crate) fn log1p(x: f64) -> f64 {
        x.ln_1p()
    }
    pub(crate) fn log2(x: f64) -> f64 {
        x.log2()
    }
    pub(crate) fn logb(x: f64) -> f64 {
        if x == 0.0 {
            return f64::NEG_INFINITY;
        }
        let e = x.abs().log2().floor();
        // 1e-320 这类 subnormal 的 log2 结果与 IEEE logb 对齐：
        // 归一化到 [1,2) 后再取指数。
        let m = x.abs() / 2f64.powf(e);
        if (0.5..1.0).contains(&m) {
            e - 1.0
        } else {
            e
        }
    }
    pub(crate) fn nextafter(x: f64, y: f64) -> f64 {
        if x.is_nan() || y.is_nan() {
            return f64::NAN;
        }
        if x == y {
            return y;
        }
        if x == 0.0 {
            return if y.is_sign_negative() {
                -f64::from_bits(1)
            } else {
                f64::from_bits(1)
            };
        }
        let mut bits = x.to_bits();
        bits = if (y > x) == (x.is_sign_negative()) {
            bits.wrapping_sub(1)
        } else {
            bits.wrapping_add(1)
        };
        f64::from_bits(bits)
    }
    pub(crate) fn remainder(x: f64, y: f64) -> f64 {
        if x.is_nan() || y.is_nan() || y.is_infinite() || x.is_infinite() && y == 0.0 {
            if y.is_infinite() && !x.is_infinite() {
                return x;
            }
            return f64::NAN;
        }
        if y == 0.0 || x.is_infinite() {
            return f64::NAN;
        }
        let n = (x / y).round_ties_even();
        let r = x - n * y;
        if r == 0.0 {
            f64::copysign(0.0, x)
        } else {
            r
        }
    }
    pub(crate) fn trunc(x: f64) -> f64 {
        x.trunc()
    }
    pub(crate) fn copysign(x: f64, y: f64) -> f64 {
        x.copysign(y)
    }
    pub(crate) fn fma(x: f64, y: f64, z: f64) -> f64 {
        x.mul_add(y, z)
    }
    pub(crate) fn frexp(x: f64) -> (f64, i32) {
        if x == 0.0 || !x.is_finite() {
            return (x, 0);
        }
        let lg = x.abs().log2();
        let mut e = lg.floor() as i32 + 1;
        let mut f = x / 2f64.powi(e);
        if f.abs() >= 1.0 {
            e += 1;
            f /= 2.0;
        } else if f.abs() < 0.5 {
            e -= 1;
            f *= 2.0;
        }
        (f, e)
    }
    pub(crate) fn ldexp(x: f64, exp: i32) -> f64 {
        if x == 0.0 || !x.is_finite() {
            return x;
        }
        // 分两步缩放，避免 exp±i32 直接溢出 powi 的参数域。
        let (big, small) = (exp / 2, exp - exp / 2);
        let s = 2f64.powi(big.clamp(-2200, 2200)) * 2f64.powi(small.clamp(-2200, 2200));
        x * s
    }
    pub(crate) fn modf(x: f64) -> (f64, f64) {
        let f = x.trunc();
        let r = x - f;
        // -0.0 符号保持：modf(-inf) 的分数部分是 -0.0
        let r = if r == 0.0 { f64::copysign(0.0, x) } else { r };
        (f, r)
    }
    pub(crate) fn remquo(x: f64, y: f64) -> (f64, i32) {
        let r = remainder(x, y);
        let mut q = ((x / y).round_ties_even() as i64 & 0x7) as i32;
        // 低三位带上 x/y 的商符号。
        if (x < 0.0) != (y < 0.0) {
            q -= 8;
        }
        (r, q)
    }
}
