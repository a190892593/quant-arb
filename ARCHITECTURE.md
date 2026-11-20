# ARBX 架构说明

> 面向「要改这份代码的人」。描述**代码实际上怎么组织、数据怎么流动、到哪里改**。
>
> 与其它文档的分工：
> - `README.md` —— 怎么跑、实现了什么、与规格有哪些差异（含限制清单）。
> - `DESIGN.md` —— 前端视觉与交互契约。
> - `docs/PRD*`、`docs/TDD*` —— 需求与技术规格（v2.0 为当前基线）。
> - `docs/产品需求规格说明书PRDv2.2.md`、`docs/产品需求规格说明书PRDv2.3.md` —— 需求基线与 v2.3 规划（含扫描引擎口径）。
> - **本文** —— 代码地图：模块职责、调用链、并发模型、持久化、改动落点。

---

## 1. 系统形态

单进程、只读链上数据、Paper 结算的本地研究系统。

- 不持有私钥、不广播交易、不提交真实 CEX 订单；`live_trading_enabled` 恒为 `false`。
- 只做**原子策略**：DEX 多池多跳在同一笔 EVM 事务内闭环。CEX–DEX 非原子双腿策略已整体移除
  （`0720352` 移除扫描链路、`cece2f1` 移除策略本体），残留的 CEX 代码只服务于「历史记录展示」。
- 执行全部是本地模拟，但**成本与门槛按真实现金成本计算**：净利 = 回款 − 投入 − Gas − 闪电贷 −
  优先费 − 延迟/MEV 准备金。AMM 手续费与滑点已内含在成交价里，不重复扣减。

## 2. 技术栈与规模

| 层 | 技术 | 规模（统计于 2026-11，HEAD `85303e1` + 工作区改动） |
|---|---|---|
| 后端 | Rust 2024 + Axum 0.8 + Tokio + REVM 43 + rusqlite(bundled) + rust_decimal | 68 个 `.rs`，29,647 行 |
| 前端 | React 19 + TypeScript + Vite 7（无 UI 库、无路由库、无状态库） | 47 个文件，13,788 行 |
| 存储 | SQLite（WAL）单文件 | 32 张表，运行态约 40 MB |
| 测试 | `cargo test --lib --tests` | 246 个（238 lib + 8 集成） |
| 接口 | REST + WebSocket + 静态托管 | 85 条路由声明（含 `/api/v1` 别名）、66 个 handler |

## 3. 目录与构建环境

```
quant-arb/
├── backend/            Rust 源码；Cargo.lock；examples/ 为一次性探针（见 §14）
├── frontend/           React 源码；dist/ 是构建产物（git 忽略）
├── config/             default.toml（Arbitrum 主实例）+ base.toml（Base 链覆盖档）
├── instances/base/     多实例工作区：config + 独立 data/ 与纸面账本
├── docs/               PRD / TDD / DEX 引擎需求与升级说明 / 合约设计 / 风控基线
├── scripts/            PowerShell：start / stop / status / build / build-ui / backup / restore / reset-paper / test
├── data/               运行态：SQLite、日志、search_config.json、source_config.json、discovered_*.json
├── backups/            一致性备份（git 忽略）
├── .build/             临时日志与诊断产物（git 忽略）
├── DESIGN.md           前端契约
├── ARCHITECTURE.md     本文
└── README.md           运行手册 + 差异清单
```

### 构建产物为什么不在这里：`D:/arbx-build/quant-arb`

`backend/.cargo/config.toml` 把 Cargo 的 `target-dir` 指向了工作区之外：

```toml
[build]
target-dir = "D:/arbx-build/quant-arb"
```

**这是本机 workaround，不是项目要求**：该配置文件被 `.gitignore` 排除，全新克隆的机器没有它，
Cargo 默认就落在 `backend/target/`（该路径同样已在 `.gitignore` 里）。

但它也不是「想当然」的。2026-11 做过一次完整的证伪尝试（目标是把 target 改回 `backend/target/`）：

| 尝试 | 条件 | 结果 |
|---|---|---|
| 1 | `cargo build`，未加载 vcvars | `cl D8050: cannot execute c1.dll`；`libsqlite3-sys` / `secp256k1-sys` / `blst` 三个 C 依赖全部失败 |
| 2 | 加载 `scripts/msvc-env.ps1` 并 `cargo clean` 这三个 crate 后重建 | 同样 D8050 |
| 3 | 同 2，且 `-j 1` 串行 | 同样 D8050 |
| 对照 A | 手工执行 cargo 打印的那条 cl 命令，输出到**工作区内**目录，加载 vcvars | **成功**，对象文件正常产出 |
| 对照 B | 同一命令，不加载 vcvars | 失败，但报的是 `C1034: stddef.h: no include path set`，不是 D8050 |

结论：**失败与 target 是否在工作区内无关**——同一条 cl 命令手工跑进工作区内的目录是成功的；
触发点在 cargo / cc-rs 启动 cl 的方式（环境、句柄继承）上，而这条路径在本机沙箱下不可用。
因此本机保留外置 target，回退尝试已撤销（`backend/target` 已清理）。

⚠️ **`cargo clean` 前必须先加载 vcvars**（`. scripts/msvc-env.ps1`）。2026-11-20 已实测：
加载 vcvars 后执行全量 `cargo clean`（清掉 4.7 GiB）再重建**成功**，三个 C crate 均正常编译；
未加载 vcvars 时才会出现 `D8050 / C1034` 一类失败。因此清理本身不必畏惧，但要带环境。

> 另注：本机目标目录/工具链曾出现不一致（cargo 1.100.0 2026-11-05 构建 vs rustc 1.100.0 2026-11-09 构建），
> 会导致 `cargo test` 的 examples 目标报 `E0786 invalid metadata files`。已验证 `cargo test --lib --tests` 可用；
> 彻底消除需重新安装/对齐工具链（未执行，属环境变更）。

要点：

- 该目录只放编译中间产物与最终二进制，源码与配置都不在里面。**但它当前不完全可再生**（见上面警告），
  所以「删掉重编」不是无代价操作。
- 当前约 **4.1 GB**，其中 `debug/arb-backend.exe` 约 28 MB。`cargo clean -p arb-backend` 只清本 crate 的产物，
  `cargo clean` 会连依赖一起清——后者在本机可能无法恢复。
- 换机器时删除该配置文件即可回到默认的 `backend/target/`；若新机器同样受限，改成任意工作区外的短路径。
- **更新二进制前要停后端**：Windows 会锁住正在运行的 `.exe`，覆盖链接会失败并可能留下不一致的增量状态
  （典型症状是后续出现 `can't find crate for std` / `invalid metadata files` 这类"像是代码坏了"的报错，
  实际只需停进程后重新编译）。

## 4. 后端分层

模块声明见 [`backend/src/lib.rs`](backend/src/lib.rs)。依赖方向自上而下，`domain` 不依赖任何 IO。

| 模块 | 职责 | 关键入口 |
|---|---|---|
| `domain/` | 纯领域模型：金额（`Decimal`）、机会、计划、执行、风控、账本、报表 | [mod.rs](backend/src/domain/mod.rs) |
| `dex/` | 准入注册表、Token–Pool 有向多重图、有界 DFS、指纹、报价数学、投入优化、Gas、M1/M3 模拟 | [multigraph.rs](backend/src/dex/multigraph.rs)、[route_search.rs](backend/src/dex/route_search.rs)、[pool.rs](backend/src/dex/pool.rs) |
| `strategy/` | 成本模型、评分、**多池多跳扫描引擎本体**、M2 复核、旧版三角/跨池（回退） | [dex_scan.rs](backend/src/strategy/dex_scan.rs)、[cost.rs](backend/src/strategy/cost.rs) |
| `risk/` | 参数基线（`HARD_STOP / SOFT_BLOCK / WARNING / PAPER_ONLY`）、覆盖解析、交易前评估、熔断 | [catalog.rs](backend/src/risk/catalog.rs)、[engine.rs](backend/src/risk/engine.rs) |
| `ledger/` | 追加式流水、10 类账户、库存与敞口、绩效与回撤 | [engine.rs](backend/src/ledger/engine.rs) |
| `paper/` | Paper 执行：DEX 原子结算、回滚也付 Gas、计划指纹 | [mod.rs](backend/src/paper/mod.rs) |
| `evm/` | JSON-RPC（free/paid 模式 + 多节点轮询 + 失败计数）、WS `newHeads` 订阅（失败降级轮询）、最小 ABI、REVM 分叉、QuoterV2 重放 | [rpc.rs](backend/src/evm/rpc.rs)、[ws.rs](backend/src/evm/ws.rs)、[fork.rs](backend/src/evm/fork.rs)、[revm_sim.rs](backend/src/evm/revm_sim.rs) |
| `feed/` | 行情快照：真实链只读（`eth_call`）+ Binance 公共盘口；`demo` 仅单元测试 | [live.rs](backend/src/feed/live.rs) |
| `cex/` | 订单簿引擎、序列号缺口失效、逐档吃单、Binance 只读适配 | [book.rs](backend/src/cex/book.rs)、[depth.rs](backend/src/cex/depth.rs) |
| `price/` | 标价：链上稳定币池 spot 主源 + Chainlink 校验 + CEX 兜底 + 三源分歧 | [oracle.rs](backend/src/price/oracle.rs) |
| `db/` | 幂等迁移 schema + 仓储 | [schema.rs](backend/src/db/schema.rs)、[repo.rs](backend/src/db/repo.rs) |
| `engine/` | `AppState`、扫描循环、启动恢复、期初库存、后台任务编排 | [scanner.rs](backend/src/engine/scanner.rs)、[seed.rs](backend/src/engine/seed.rs) |
| `api/` | Axum 路由、handler、WebSocket、静态托管 | [mod.rs](backend/src/api/mod.rs)、[handlers.rs](backend/src/api/handlers.rs) |
| `search_config` `source_config` | 两个运行态注册表：搜索参数/Token 名单、数据源清单 | [search_config.rs](backend/src/search_config.rs)、[source_config.rs](backend/src/source_config.rs) |
| `metrics/` | 归一化指标事件：Decimal 精度、P50/P95/P99、环形窗口 + 落库（V22-101） | [metrics.rs](backend/src/metrics.rs) |
| `attribution/` | 12 类失败归因（PRD §11.3），只按已有事实分类（V22-105/307） | [attribution.rs](backend/src/attribution.rs) |
| `bid/` | EV 决策器：P 缺失即 `INSUFFICIENT_DATA`（V22-605） | [bid.rs](backend/src/bid.rs) |
| `competition/` | 竞争强度分类：无数据即 `INSUFFICIENT_DATA`（V22-501） | [competition.rs](backend/src/competition.rs) |

## 5. 一次扫描的完整链路

入口：[`engine::scanner::scan_once`](backend/src/engine/scanner.rs)（默认每 1.5s 一轮，
扫描器初始 `STOPPED`，需显式启动）。

```
feed 快照 (MarketData，不可变)                      feed::live::sync
  └─ Token/Pool 准入                                dex::registry
       └─ 建 Token–Pool 有向多重图                   dex::multigraph
            └─ 预算约束的有界 DFS（2–5 跳）           dex::route_search
                 └─ 有向指纹去重                      dex::fingerprint
                      └─ 逐路线投入优化 + 精确报价      dex::optimizer + dex::pool
                           └─ Gas / 成本 / 净利        dex::gas + strategy::cost
                                └─ M1 一致性 + M3 压力  dex::sim
                                     └─ 路线级门禁      strategy::dex_scan::gate_route
                                          └─ 风控评估   risk::RiskEngine::evaluate
                                               └─ Opportunity（ACTIONABLE / WATCH / REJECTED）
                                                    └─ Paper 执行   paper::PaperEngine
                                                         └─ 记账    ledger::LedgerEngine
                                                              └─ SQLite + WebSocket + 前端轮询
```

关键约束：

- **报价只读同一份快照**：一次同步生成一份 `MarketData`，扫描期间池状态不会被改写。
- **门禁在生成机会之前**：不满足准入/一致性/容量/门槛的路线不会变成 `ACTIONABLE`，只带拒绝原因留档。
- **风控在后端**：前端开关只是发起请求，真正拦截在 `RiskEngine::evaluate`；`evaluate` 内部自行
  重算门槛（`min_profit_for`），不接受调用方传入的门槛值。
- **执行路径必须复核状态**：扫描循环只取 `status == Actionable`；API 执行还额外校验新鲜度与熔断暂停。

## 6. 并发模型

单进程内四类 Tokio 任务（[`main.rs`](backend/src/main.rs)）：

| 任务 | 周期 | 作用 |
|---|---|---|
| `engine::spawn_feed` | `[live] refresh_ms`（2s） | 拉链上池状态与盘口，替换 `MarketData` 快照；失败只降健康度，不清零 |
| `engine::spawn_discovery` | `[discovery]`（默认 10min） | 从聚合器拉候选池 → 回链上校验 → 写 `discovered_pools.json` 与白名单 |
| `engine::scanner::spawn_m2` | 60s | 用 REVM 在报价区块上复核一条全 V3 路径；与扫描循环解耦 |
| `engine::scanner::spawn` | 1.5s | 扫描循环：扫描 → 风控 → （可选）执行 → 落库 → 广播 |

共享状态集中在 [`AppState`](backend/src/engine/mod.rs)：

- `market: RwLock<MarketData>` —— 读多写少，扫描只读快照。
- `ledger / paper: Mutex<...>` —— 写路径串行化，账本不允许并发改。
- `opportunities / executions / risk_events / dex_routes ...: RwLock<Vec<_>>` —— 内存窗口，落库在旁路。
- `events: broadcast::Sender<WsEvent>` —— 推送 Opportunity / Execution / RiskEvent / Scanner / Route /
  Graph / Simulation / Book / Ledger / System 十类事件；WebSocket 只在连接时推一次快照，之后走增量。

## 7. 数据与持久化

- 32 张表，分四组：
  - **链与资产**：`chains` `tokens` `pools` `pool_states` `dex_protocols`
  - **市场与机会**：`opportunities` `dex_routes` `route_fingerprints` `simulations` `scan_runs`
  - **执行与账本**：`trade_plans` `executions` `execution_events` `cex_fills` `unwind_events`
    `accounts` `ledger_entries` `inventory_snapshots`
  - **风控与治理**：`risk_events` `risk_params` `risk_overrides` `system_controls` `system_configs`
    `audit_jobs` `audit_reports` `scanner_state`
- **金额与数量一律 TEXT 存 `Decimal` 原文**，禁止浮点。`*_json` 列保存完整领域对象，关键字段单独建列供查询。
- **账本只追加**：余额由流水聚合，`balance_after` 固化便于审计；权益 = 资产类账户余额 × 价格。
- 迁移是幂等 `CREATE TABLE IF NOT EXISTS` + `ALTER` 迁移块，见 [`db/schema.rs`](backend/src/db/schema.rs)；
  新增列务必保持幂等，否则老库起不来。
- 备份用 SQLite `VACUUM INTO`（一致性副本）；恢复要求后端先停止。

## 8. 接口面

- 全部业务接口挂在 `/api` 下，便于与静态文件共存；`/api/v1/*` 是 PRD §15 的契约别名，
  **复用同一批 handler**（保持单一实现，避免两套逻辑漂移）。
- `/ws` 推送增量事件；`/healthz` 供探活。
- 前端构建产物由后端同源托管（`server.ui_dir = "frontend/dist"`），未构建时根路径降级为一句提示。
- 主要分组：系统 / 搜索配置 / 数据源 / 发现 / 链与 DEX / CEX 行情 / 扫描控制 / 路线与模拟 /
  机会与执行 / 账本与风控 / 审计 / 合约状态 / 指标（`/api/metrics/summary|events|report`）/
  路线覆盖（`/api/dex/coverage`）/ 竞争与竞价（`/api/competition/overview`、`/api/competition/bidding/simulate`）/
  RPC 状态（`/api/rpc/status`：模式、端点失败计数、WS 端点）。

## 9. 前端结构

```
frontend/src/
├── App.tsx          哈希路由表 + 布局；URL(kebab) → 视图 key(camel) 的映射也在这里
├── components/      Shell(顶栏/侧栏)、nav.ts(导航表)、搜索配置抽屉、机会抽屉、命令面板
├── views/           每个路由一个视图（16 个）
├── data/            api.ts（后端契约 → 领域模型映射）、index.ts（轮询与原地合并）、dataset.ts（演示数据）
├── ui/              Chip / Drawer / SortStack / Toast
├── charts/          LineChart / Spark / Waterfall（手写 SVG，无图表库）
└── styles/          tokens.css（设计令牌）、app.css、views.css
```

两个容易被误解的设计：

1. **没有路由库**：`location.hash` + `hashchange`；旧链接（`#/routes` `#/scanner-cex` `#/scanner`）
   全部重定向到 `#/scanner-dex`。
2. **数据层"原地合并"而不是替换**：演示数据在模块加载时就位，`refresh()` 每 3s 拉 `/api/*` 后**清空并回填**
   同名数组 / 逐字段覆盖对象，因此各视图里模块级的 `const ROWS = data.opps` 引用始终有效。
   轮询用 `useSyncExternalStore`，页面隐藏时暂停。后端不可达时顶栏显示 `DEMO · OFFLINE`，不伪造数据。

## 10. 配置与运行态

| 位置 | 内容 | 生效方式 |
|---|---|---|
| `config/default.toml` | 主实例配置（链、池、Token、引擎参数、风控联动项） | 重启 |
| `config/base.toml` + `ARBX_PROFILE=base` | Base 链覆盖档 | 重启（独立端口 8792、独立数据目录） |
| `[live] rpc_mode` / `ARBX_RPC_MODE` | RPC 模式：`free` 公共多节点轮询 / `paid` 付费优先、免费兜底 | 重启 |
| `data/search_config.json` | Token 白/黑名单 + 27 项搜索参数 | **保存即生效**；Token 名单变更需重启重建 feed |
| `data/source_config.json` | 27 个数据源的启停/优先级/限速/Key | 每轮发现任务读取，保存即生效 |
| `data/discovered_*.json` | 自动发现的池与价格 | feed 定期热加载 |

环境变量：`ARBX_PORT`、`ARBX_WORKSPACE`、`ARBX_PROFILE`、`ARBX_LOG`、`COINGECKO_API_KEY`。

**门槛类参数只能收紧**：风控覆盖 `RiskEngine::set_override` 会拒绝把 Max 型调大、Min 型调小；
延迟/MEV 准备金由 `search_config::MIN_RESERVE_BPS` 硬夹紧，配置与界面都只能上调。

## 11. 被测试锁住的不变式

改代码时如果动了下面这些，测试会直接失败——这是有意的：

| 不变式 | 锁在哪 |
|---|---|
| 最低净利润 = `max(2 USDC, 名义 × 10 bps)`，且为 `HARD_STOP/Min` | `risk::catalog::tests` |
| 任何「下限型」风控参数不得为 0 | `risk::catalog::tests::no_floor_param_is_disabled` |
| 延迟/MEV 准备金不得被配置或界面清零 | `search_config::tests::reserves_cannot_be_zeroed` |
| 引擎默认准备金与 JSON 默认值同源 | `search_config::tests::default_reserves_match_engine_baseline` |
| DEX 回滚也要付 Gas（净利为负） | `tests/paper_flow.rs::dex_revert_still_pays_gas` |
| `equity − start_equity == 净利` | `ledger` 与绩效相关测试 |
| 覆盖不得放宽（Max 不可调大、Min 不可调小） | `risk::engine::tests` |
| 成本明细可加总，合计由明细求和 | `strategy::cost::tests` |

## 12. 测试布局

```
cargo test --lib --tests     # 246 个：238 lib + 8 集成
```

权重在数学与风控：`dex/math`（22）、`risk/engine`（20）、`cex/depth`（16）、`ledger/engine`（13）。
集成只有两个：`tests/paper_flow.rs`（DEX 成功/回滚/资金不足/计划过期/方向）、
`tests/multihop_scan.rs`（合成市场端到端：指纹唯一、优化自洽、增量复用、落库重读）。

## 13. 常见改动落点

| 想改什么 | 改哪里 | 注意 |
|---|---|---|
| 风控门槛 / 新增规则 | `risk/catalog.rs` + `risk/engine.rs` | 不要改 `config/default.toml` 来放宽；改基线要同步文档与测试 |
| 搜索参数默认值 | `strategy/dex_scan.rs` 与 `search_config.rs` | 两处必须同源，有测试比对 |
| 加池 / 加 Token | `config/default.toml` 的 `[[live.extra_pools]]` / `[[live.tokens]]` | 或交给自动发现写入白名单 |
| 新数据源 | `source_config.rs` 注册表 | 默认关闭需 Key 的源 |
| 新 API | `api/mod.rs` 路由 + `api/handlers.rs` | v1 别名复用同一 handler |
| 新前端页 | `views/` + `App.tsx` 的 `ROUTES`/`URL_ROUTE` + `components/nav.ts` | 三处都要加，否则页面不可达 |
| 新表 / 新列 | `db/schema.rs` 的 `MIGRATIONS` | 必须幂等（`IF NOT EXISTS` / `ALTER`） |
| 新 WS 事件 | `engine::WsEvent` + `api/ws.rs` + 前端 `data/api.ts` | 事件名是前后端契约 |

## 14. 已知限制与技术债

1. **`backend/examples/*` 编译不过**：12 个开发期探针引用了已删除的 `strategy::cex_dex` 与旧版
   `ForkCall` 字段。所以测试命令是 `--lib --tests`，不是裸 `cargo test`。清理后可以恢复。
2. **M2 只覆盖全 V3 路径**：混合路线（含 V2/Stable）最高只能是 M1，生命周期不会出现 `VERIFIED`。
3. **行情是单次 `eth_call` 快照**：V3 只按当前区间报价（不扫 tick 位图），公共 RPC 延迟 1–4s，
   因此 DEX 报价年龄预算放宽到 8s（原基线 3s 在公共 RPC 下不可达）。
4. **历史回放未实现**：审计只统计本机已记录数据，样本不足直接给 `WATCH`，不编造缺失区间。
5. **执行合约只有设计**：无 Solidity 代码、未部署，`/api/contract/status` 明确返回 `deployed: false`。
6. **`paper::build_plan` 的 `min_profit_usd` 恒为 0**：该字段目前不参与任何判断（真实门槛在
   `RiskEngine::evaluate`），但字段名容易误导，属待清理项。
7. **两个实例互不感知**：Arbitrum(8791) 与 Base(8792) 各自持有数据库与纸面账本，没有跨实例对账。
