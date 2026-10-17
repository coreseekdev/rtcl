// event-loop.js —— wasm 主线程形态的事件循环心跳（docs/ROADMAP.md §3）。
//
// wasm 里没有自发的事件循环：`after ms script` 到点只是进队列，要有
// 人泵（Worker 形态自带循环，主线程形态靠这里）。周期性的空
// dispatch_event 把到点的 after/队列事件冲出；pump 起时已到点的
// 事件全部执行（新调度的下一次不会在同一次泵里再冲——语义见
// rtcl-core event.rs），所以心跳间隔决定时间分辨率，不会链爆。
//
// 返回 stop 函数。宿主应把心跳错误（回调脚本抛错等）接到 onError，
// 别静默（通常是 shell 的 error 通道）。

export function startHeartbeat(rtcl, { intervalMs = 50, onError } = {}) {
  const id = setInterval(() => {
    try {
      rtcl.dispatch_event('');
    } catch (e) {
      if (onError) onError(String((e && e.message) || e));
    }
  }, intervalMs);
  return () => clearInterval(id);
}
