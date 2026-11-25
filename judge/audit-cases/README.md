# audit-cases — 审计遗留项的复现脚本

`specs/CODE-AUDIT-20261123.md` 里还没修的几项，各有一个最小复现。
每个脚本都能在任意 Tcl 解释器下直接跑，输出一行一项。

```sh
bash judge/audit-cases/rr.sh judge/audit-cases/<脚本>.tcl
```

`rr.sh` 用四个解释器各跑一遍，每个一行：`86` 是系统的 tclsh 8.6.17，
`91` 是仓内构建的 tclsh 9.1.0（`.refer/tcl910/inst/bin/tclsh9.1`，
**现在的 oracle**），`vm` 是 rtcl 字节码引擎，`tree` 是
`rtcl --no-bytecode`。两个 tclsh 不一致的行是 8.6 与 9.1 的语义差，
不是 rtcl 的缺陷；rtcl 与 `91` 不一致的行才是。

| 脚本 | 审计编号 | 看什么 |
|---|---|---|
| `ns-chain-86-vs-91.tcl` | C52、C53 | 命名空间体里的裸名变量。8.6 回落到全局变量，9.1 不回落：S1、S3、S7、S8、S9、S11、S14、S15 两个 tclsh 不同，rtcl 现在跟的是 8.6。S5、S6（proc 里执行的体有自己的变量帧，C49）三个一致 |
| `locals-order.tcl` | C42 的余项 | proc 里 `info locals`/`info vars` 的顺序。两个 tclsh 一致，rtcl 两个引擎各不相同 |
| `var-errorcodes.tcl` | C50、C51 | 29 种变量读写失败的 errorCode。rtcl 只在 `incr-read-trace-fail`、`lappend-read-trace-fail`、`unset-trace-fail2` 上与两个 tclsh 不同 |
| `bench-ns-body.tcl` | P16 | 命名空间体里 1500 万次裸名读写。整段墙钟时间：tclsh 2.6–3.5 秒，rtcl 约 40 秒 |

改名或新增脚本时更新这张表；编号以审计文档为准。
