// Worker 面轻验证（node worker_threads 链，真机以浏览器为准）：
// ① rtcl-worker.js 启动序列含 ABI 版本校验（docs/ABI-OPT-NOTES.md
//    §3a）——ready 消息 = 校验通过；② ready 后 exec 探针一式。
// 运行：node test-worker.mjs（先 ./build.sh）。退出码 0 = 全过。
import { Worker } from 'node:worker_threads';
import { setTimeout as sleep } from 'node:timers/promises';

const workerUrl = new URL('./www/rtcl-worker.js', import.meta.url);
const w = new Worker(workerUrl, { type: 'module' });

let pass = 0, fail = 0;
let done = false; // terminate() 使 worker 以码 1 退出——成功收尾后不再判失败
const ok = (cond, label, detail) => {
    if (cond) { pass++; console.log(`  ok  ${label}`); }
    else { fail++; console.error(`FAIL  ${label}`, detail ?? ''); }
};

const timer = sleep(60000).then(() => {
    console.error('FAIL  worker boot timeout (no ready message in 60s)');
    process.exit(1);
});

w.on('message', async (m) => {
    if (m.type === 'init-error') {
        console.error('FAIL  init-error:', m.error);
        process.exit(1);
    }
    if (m.type === 'ready') {
        ok(true, `worker ready（ABI 版本校验通过）fs=${m.fs}`);
        // exec 探针（ready 前投递的消息由 BOOT_QUEUE 承接——直接在
        // ready 后投，走 booted 分派路径）。
        w.postMessage({ id: 1, script: 'expr {1 + 2}' });
        w.postMessage({ id: 2, script: 'set s 0; for {set i 1} {$i <= 100} {incr i} { set s [expr {$s + $i}] }; set s' });
    }
    if (m.id === 1) ok(m.ok === true && m.result === '3', 'exec: expr arithmetic', JSON.stringify(m));
    if (m.id === 2) {
        ok(m.ok === true && m.result === '5050', 'exec: for loop 5050', JSON.stringify(m));
        clearTimeout(timer);
        console.log(`\n${pass} passed, ${fail} failed`);
        done = true;
        await w.terminate();
        process.exit(fail > 0 ? 1 : 0);
    }
});
w.on('error', (e) => { console.error('FAIL  worker error:', e); process.exit(1); });
w.on('exit', (c) => { if (!done) { console.error(`FAIL  worker exited early (${c})`); process.exit(1); } });
