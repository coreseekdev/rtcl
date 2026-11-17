//! lazy-string（§6）的 wasm 侧：JsStringBacking 注册 + 边界零转码。
//!
//! **池**：`Vec<Option<Slot>>` + 空闲栈，槽位带引用计数（rtcl-vm 的
//! `InternalRep::HostStr` acquire/release 驱动）；`js_sys::JsString` 是
//! externref，池表保活，JS 引擎管内存——wasm 侧只有句柄与槽位簿记。
//!
//! **lone surrogate 防线**（§6，入池唯一过滤器）：JS 串可含孤立代理，
//! rtcl 串不能。所有 JS→Tcl 的字符串入口统一走 [`value_from_js`]：
//! `is_valid_utf16()` 一次校验——合法标量值序列才入池（`HostStr` 形态），
//! 含孤立代理的串立即经 `String::from` 有损转码（孤立代理 → U+FFFD）+
//! 原生字节形态进值，永不以 HostStr 存在。白名单的安全性论证全部建立
//! 在这条池不变量上。
//!
//! **Tcl→JS 穿越面**（[`value_to_js`]）：HostStr 句柄直取池中 JsString
//! （零转码，收益本体）；原生值保持既有 `String()` 转码——不自动反向
//! 缓存：eval 结果/转发参数几乎都是一次性值，入池缓存只会白付簿记与
//! 池翻新（§6 的反向 Dual 形态保留在 `Value::to_host_str`，留给显式
//! 复用场景）。

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::JsString;
use wasm_bindgen::prelude::*;

use rtcl_core::host_string::{self, HostStringBacking, HostStrBytes};
use rtcl_core::Value;

struct Slot {
    s: JsString,
    rc: u32,
}

/// JsString 池（rtcl-vm `HostStringBacking` 的 wasm 实现）。
#[derive(Default)]
pub struct JsStringPool {
    slots: RefCell<Vec<Option<Slot>>>,
    free: RefCell<Vec<u32>>,
}

thread_local! {
    /// 进程级单例：所有 RtclHandle 共用一池（值/池生命周期绑定在值上，
    /// 解释器实例数无关）；句柄另存一份 Rc 供边界零转码取串。
    static POOL: RefCell<Option<Rc<JsStringPool>>> = const { RefCell::new(None) };
}

/// 注册背板（幂等）：`RtclHandle::new` 调一次；重复调用复用同实例。
///
/// 运行时开关（§8 `lazy-string` 的 wasm 形态）：页面在**构造 handle 前**
/// 设 `globalThis.RTCL_DISABLE_LAZY_STRING = 1` 即回到全转码基线（不注
/// 册 → HostStr 永不出现 → 所有边界走既有 String() 转码）。默认 on
/// （§6：rtcl-wasm 默认 on）。开关同时是 A/B 量化的干净手段：同一 pkg
/// 翻一个标志对比，免双 pkg。
pub fn ensure_backing() {
    #[cfg(target_arch = "wasm32")]
    {
        if js_sys::Reflect::has(&js_sys::global(), &JsValue::from_str("RTCL_DISABLE_LAZY_STRING"))
            .unwrap_or(false)
        {
            return;
        }
    }
    POOL.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let pool = Rc::new(JsStringPool::default());
            host_string::register_host_string_backing(pool.clone());
            *slot = Some(pool);
        }
    });
}

/// 取池（边界取串用；未注册 = 不可能，RtclHandle::new 保证）。
fn with_pool<R>(f: impl FnOnce(&JsStringPool) -> R) -> Option<R> {
    POOL.try_with(|slot| slot.borrow().as_deref().map(f)).ok().flatten()
}

impl JsStringPool {
    /// JS 原生串入池（入池前提：调用方已过 `is_valid_utf16` 校验）。
    fn insert_js(&self, s: JsString) -> u32 {
        let mut slots = self.slots.borrow_mut();
        let mut free = self.free.borrow_mut();
        if let Some(idx) = free.pop() {
            let i = idx as usize;
            slots[i] = Some(Slot { s, rc: 1 });
            idx
        } else {
            let idx = slots.len() as u32;
            slots.push(Some(Slot { s, rc: 1 }));
            idx
        }
    }

    /// 池中取 JsString（克隆 = JS 引擎引用计数 +1，不触池簿记）。
    fn get_js(&self, idx: u32) -> Option<JsString> {
        self.slots
            .borrow()
            .get(idx as usize)
            .and_then(|s| s.as_ref().map(|slot| slot.s.clone()))
    }
}

impl HostStringBacking for JsStringPool {
    /// JsOnly → Dual 物化：池串已过入口校验（合法 UTF-16），`String::from`
    /// 无损（UTF-16→UTF-8，转码恰好一次——白名单外首次字节语义）。
    fn to_bytes(&self, idx: u32) -> HostStrBytes {
        let s = self.get_js(idx).unwrap_or_default();
        HostStrBytes::from_slice(String::from(&s).as_bytes())
    }

    /// 免物化深层相等（JsString `==` = UTF-16 码元序列比较；池内容恒为
    /// 合法标量值 → 与 UTF-8 字节相等双射）。
    fn equal(&self, a: u32, b: u32) -> bool {
        match (self.get_js(a), self.get_js(b)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// 免物化拼接（JS string concat）：合法串「拼接→转码 ≡ 转码→拼接」。
    fn concat(&self, a: u32, b: u32) -> u32 {
        let sa = self.get_js(a).unwrap_or_default();
        let sb = self.get_js(b).unwrap_or_default();
        self.insert_js(sa.concat(&sb))
    }

    /// 反向入池（Tcl→JS 双缓存的显式形态，`Value::to_host_str` 用）。
    fn insert(&self, bytes: &[u8]) -> u32 {
        self.insert_js(JsString::from(String::from_utf8_lossy(bytes).into_owned()))
    }

    /// 免物化字节空判（合法串：UTF-16 空 ⇔ UTF-8 空）。
    fn is_empty(&self, idx: u32) -> bool {
        self.get_js(idx).map(|s| s.length() == 0).unwrap_or(true)
    }

    fn acquire(&self, idx: u32) {
        if let Some(Some(slot)) = self.slots.borrow_mut().get_mut(idx as usize) {
            slot.rc += 1;
        }
    }

    fn release(&self, idx: u32) {
        let mut slots = self.slots.borrow_mut();
        let i = idx as usize;
        let freed = match slots.get_mut(i) {
            Some(Some(slot)) => {
                slot.rc -= 1;
                slot.rc == 0
            }
            _ => false,
        };
        if freed {
            // 归零回收：槽位清空（JsString 交还 JS 引擎），索引进空闲栈。
            slots[i] = None;
            drop(slots);
            self.free.borrow_mut().push(idx);
        }
    }
}

/// 已确认字符串的 JsValue 落值（校验 → 入池 / 有损兜底）。
pub fn js_string_to_value(s: &JsString) -> Value {
    if s.is_valid_utf16() {
        with_pool(|p| p.insert_js(s.clone()))
            .map(Value::from_host_str)
            .unwrap_or_else(|| Value::from_str(&String::from(s)))
    } else {
        // 孤立代理：立即转码 + U+FFFD 兜底，永不以 HostStr 形态存在。
        Value::from_str(&String::from(s))
    }
}

/// JS → Tcl 的统一入口（§6 lone surrogate 防线 + 免转码入池），替代
/// 既有 `js_value_to_result` 的 `String()` 双跳转码：
///
/// - 字符串值：`is_valid_utf16()` 校验 → 合法入池（`HostStr`，零转码）；
///   含孤立代理则有损转码（U+FFFD 兜底）走原生值。
/// - 非字符串值：保持既有 `String()` 兜底语义（结果同样是 JS 字符串，
///   同样过校验入池）。
pub fn value_from_js(cmd_name: &str, v: JsValue) -> Result<Value, rtcl_core::Error> {
    if v.is_string() {
        // JsValue → JsString 不转码（同一 externref 重包）。
        return Ok(js_string_to_value(&JsString::unchecked_from_js(v)));
    }
    let string_fn = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("String")).map_err(
        |e| {
            rtcl_core::Error::runtime(
                format!("global String() not available: {:?}", e),
                rtcl_core::error::ErrorCode::Generic,
            )
        },
    )?;
    let s = js_sys::Function::from(string_fn)
        .call1(&JsValue::NULL, &v)
        .map_err(|e| {
            rtcl_core::Error::runtime(
                format!(
                    "JS command '{}' returned an unstringifiable value: {}",
                    cmd_name,
                    crate::js_command::js_error_message(&e)
                ),
                rtcl_core::error::ErrorCode::Generic,
            )
        })?;
    Ok(if s.is_string() {
        js_string_to_value(&JsString::unchecked_from_js(s))
    } else {
        Value::empty()
    })
}

/// Tcl → JS 穿越面：HostStr 句柄直取池串（零转码，收益本体）；
/// 原生值走既有 `String()` 转码（一次性值不反向缓存，见模块注释）。
pub fn value_to_js(v: &Value) -> JsValue {
    if let Some(idx) = v.host_str_handle() {
        if let Some(s) = with_pool(|p| p.get_js(idx)) {
            return s.into();
        }
    }
    JsValue::from_str(&v.to_string())
}
