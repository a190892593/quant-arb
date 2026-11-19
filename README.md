# ARBX · 原子套利与 CEX–DEX 对冲交易系统（第一阶段实现）

本仓库是 `doc/` 下四份规格的**可运行实现**：本地优先（Local First）、Paper Trading、
不广播链上交易、不提交真实 CEX 订单。年化收益目标不是验收承诺，系统也不保证盈利。

- DEX 多池多跳扫描引擎需求：`doc/DEX 扫描引擎需求文档.md`
- 本次扫描引擎升级说明：`doc/DEX扫描引擎技术升级说明.md`
- 需求：`doc/产品需求规格说明书PRDv1.2.md`
- 技术：`doc/总体技术设计说明书TDDv1.2.md`
- 合约：`doc/执行合约详细设计 v1.0.md`
- 风控：`doc/风控参数基线与熔断规则v1.0.md`
- 前端设计契约：`DESIGN.md`

## 1. 快速开始

最简单：双击 `start.cmd`（内部用 `powershell -ExecutionPolicy Bypass -File` 调用脚本，
不需要修改系统执行策略）。其余操作用 `start.cmd` / `stop.cmd` / `status.cmd` 或对应 `.ps1`。

```powershell
# 若直接运行 .ps1，请先为当前进程放行一次：
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force

.\scripts\build.ps1          # 首次构建（自动加载 MSVC 环境）
.\scripts\start.ps1 -Open    # 启动后端并打开界面
.\scripts\status.ps1         # 查看扫描/账本/风控状态
.\scripts\stop.ps1           # 停止（数据保留）
.\scripts\backup.ps1         # 一致性备份到 backups/
.\scripts\restore.ps1 -File .\backups\arbx-20261119-200832.db
.\scripts\reset-paper.ps1    # 重置纸面账户（保留代码与配置）
.\scripts\test.ps1           # 全部测试
```

- 界面：<http://127.0.0.1:8791/>（后端同时托管 `web/` 静态文件，前后端同源）
- API：`http://127.0.0.1:8791/api/*`，WebSocket：`ws://127.0.0.1:8791/ws`
- 端口默认 8791（本机 8787 已被其他服务占用）；可用 `ARBX_PORT` 覆盖
- 日志：`data/server.out.log` / `data/server.err.log`
- 数据库：`data/arbx.db`（SQLite，WAL 模式）

启动后扫描器默认处于 `STOPPED`：这是有意的安全默认值，需要在界面或
`POST /api/scanner/start` 显式启动扫描。`.\scripts\start.ps1` 之后的
`status.ps1` 会显示真实状态。

## 2. 实现范围

| PRD 交付项 | 实现位置 | 状态 |
|---|---|---|
| 项目配置、链注册、协议注册 | `backend/src/config.rs`、`feed/demo.rs`、`db/` | 完成 |
| DEX 池状态与报价 | `backend/src/dex/`（V2 / V3 集中流动性 / StableSwap / Weighted） | 完成 |
| DEX 多池多跳扫描（v2） | `backend/src/dex/` + `strategy/dex_scan.rs`（准入 / 多重图 / 有界 DFS / 指纹 / 投入优化 / M1+M3 / 增量） | 完成（M2 未接入，见 §7） |
| CEX 公共行情与订单簿 | `backend/src/cex/`（快照+增量、缺口失效、深度成交、Binance 只读适配） | 完成 |
| 历史审计数据导入/校验/重放 | `arb-backend audit` CLI + `POST /api/audit/jobs`（只审计已记录数据） | 部分（见 §7） |
| DEX–DEX 套利扫描 | 新引擎 `strategy/dex_scan.rs`（2–5 跳，默认）；旧 `strategy/dex_arb.rs` 仅作回退 | 完成 |
| CEX–DEX 对冲扫描 | `strategy/cex_dex.rs`（双向、盘口深度、延迟/补偿成本） | 完成 |
| 统一机会评分与盈亏推演 | `strategy/cost.rs`、`strategy/scorer.rs`、`domain/opportunity.rs` | 完成 |
| 统一 Paper Trading 引擎 | `backend/src/paper/`（DEX 模拟、CEX 深度成交、双腿状态机、补偿） | 完成 |
| 策略账本与资金管理 | `backend/src/ledger/`（10 类账户、追加式流水、库存、对账、绩效） | 完成 |
| 风控及熔断中心 | `backend/src/risk/`（49 项参数、覆盖只允许更严格、熔断窗口） | 完成 |
| 执行合约设计与测试规范 | `doc/执行合约详细设计 v1.0.md`（设计层交付） | 未编码（见 §7） |
| 前端仪表盘 | `web/`（13 视图 + 抽屉，已接入 `/api`） | 完成 |
| 本地启动/停止/备份/恢复 | `scripts/` | 完成 |
| 审计与 Paper 报告 | `/api/audit/*`、`/api/paper/performance`、`arb-backend audit` | 完成 |

## 3. 架构

```
backend/src/
├── domain/       领域模型（金额 Decimal、机会、计划、执行、风控、账本、报表）
├── dex/          准入、Token–Pool 多重图、有界搜索、指纹、报价数学、投入优化、Gas、M0–M3 模拟
├── cex/          订单簿引擎、深度成交模拟、Binance 公共行情适配器
├── strategy/     成本模型、评分、三角/跨池/CEX–DEX 搜索、风控判定编排
├── risk/         49 项风控参数基线、覆盖解析、交易前评估、熔断器
├── ledger/       追加式资金账本、库存与跨场所敞口、绩效报告
├── paper/        Paper 执行：DEX 模拟、CEX 深度成交、双腿状态机、补偿
├── feed/         行情来源（本地合成市场，确定性可复现）
├── db/           SQLite schema 与仓储（替代独立 PostgreSQL 服务）
├── engine/       应用状态、扫描编排、启动恢复、期初库存注入
└── api/          Axum HTTP + WebSocket + 静态文件托管
```

单进程同时承担 API、扫描器与 Paper 结算（TDD §2.1 允许），通过 Tokio 任务编排；
历史审计保留独立 CLI 入口（`arb-backend audit`）。

### 数据流

```
feed.tick → MarketView → StrategyEngine.scan
        → 逐跳精确报价 + 成本模型 + 评分
        → RiskEngine.evaluate（硬限制在此拦截）
        → Opportunity（ACTIONABLE / WATCH / REJECTED + 拒绝原因）
        → PaperEngine.build_plan → execute
        → LedgerEngine.post_batch（资产账户 ± / 费用账户 +）
        → ExecutionRecord（状态机 + 双腿 + 补偿 + 账本关联）
        → SQLite + WebSocket 推送 + 前端
```

## 4. 关键语义（与文档对齐）

- **原子 vs 非原子**：DEX 三角/跨池在同一笔 EVM 事务内闭环；CEX–DEX 明确不是原子交易，
  两腿独立成交、可部分成交、可一方回滚，必须补偿（`paper/mod.rs`）。
- **成交以实际数量为准**：IOC 部分成交后，第二腿按实际成交量重算；FOK 未全成视为未成交。
- **状态未知 ≠ 未成交**：超时先标记 `ORDER_STATUS_UNKNOWN`，在查询窗口内核对，
  确认未成交后才结束流程，**不重发**。
- **回滚也要付 Gas**：DEX 回滚记录 `DEX_REVERTED` 且 `GAS_EXPENSE` 入账，
  净利为负（集成测试 `dex_revert_still_pays_gas`）。
- **成本逐项可加总**：毛利 − 成本 = 净利；`Opportunity.costs` 明细与合计同源。
- **权益可核对**：`equity − start_equity == 净利`（`/api/paper/performance`，
  集成测试与运行验证均为 0 差异）。
- **覆盖不得放宽**：`RiskEngine::set_override` 拒绝把 Max 型调大、Min 型调小。
- **风控在后端**：前端开关只发起请求，真正拦截在 `RiskEngine::evaluate`。
- **DEX 成本口径**：AMM 手续费与滑点已含在成交价中，净利润只再扣 Gas、闪电贷、优先费、
  延迟/MEV 准备金等真实现金成本（旧实现重复扣减，新引擎已修正；见
  `doc/DEX扫描引擎技术升级说明.md` §5）。
- **模拟等级诚实**：机会 `model_level` 最高为 M1（状态一致性），并附 M3 压力结果；
  本机未接入 REVM，M2 返回 `UNAVAILABLE`，生命周期不会出现 VERIFIED。
- **生命周期与状态并行**：`status` 面向前端可执行性，`lifecycle` 面向审计；
  路线级门禁（Token/Pool 准入、同区块状态、M1 通过、容量）与 49 项风控合并判定。

### 风控评估顺序（`risk/engine.rs`）

暂停状态 → 数据质量 → 报价区块差/年龄 → 盘口有效性/年龄/费率未知/深度占用/深度倍数
→ 最低净利润（DEX `max(2, 10bps)`；CEX–DEX `max(5, 30bps)`）→ Gas 占比与绝对上限
→ 价格冲击 → 库存充足性 → 闪电贷费用已知 → 日亏损/策略亏损/回撤 → 单 Token 敞口。

## 5. 数据与持久化

- 表结构对应 TDD §10 的主要实体：`chains`、`tokens`、`pools`、`pool_states`、
  `cex_instruments`、`opportunities`、`trade_plans`、`executions`、`cex_fills`、
  `execution_events`、`unwind_events`、`accounts`、`ledger_entries`、`risk_events`、
  `risk_params`、`risk_overrides`、`system_controls`、`audit_jobs`、`audit_reports`、
  `inventory_snapshots`、`scanner_state`、`dex_routes`、`route_fingerprints`、
  `simulations`、`scan_runs`（后四张表为多池多跳引擎新增）。
- 金额与数量一律以 TEXT 保存 `Decimal` 原文，禁止浮点。
- 账本流水只追加；余额由流水聚合，`balance_after` 固化便于审计。
- 期初资金分配写入 `DEPOSIT`/`TRANSFER` 流水（`ref_id = seed-inventory`），可追溯。
- 备份使用 SQLite `VACUUM INTO`（一致性副本），恢复要求后端先停止。

## 6. 前端

实时界面是 `frontend/`（React 19 + TypeScript + Vite，契约见 `DESIGN.md`），构建产物由后端
同源托管（`ui/`，默认 `server.ui_dir = "ui"`）。前端每 2.5s 轮询 `/api/*` 并映射为领域模型，
顶栏显示 `LIVE API`；后端不可达时明确降级为离线状态，不伪造数据。

本次新增「路由搜索中心」（`#/routes`）：展示图层统计（池/边/Token）、候选路线、重算/复用、
搜索耗时与截断标记，以及每条路线的逐腿明细、指纹、投入金额优化曲线、M1/M3 模拟结果和拒绝原因。
扫描器「启动/停止」、风控「全局暂停/恢复」、机会抽屉「生成模拟执行」都调用真实 API。

## 7. 与规格的差异与限制（必须阅读）

1. **数据库**：TDD 建议 PostgreSQL；本机未安装 PostgreSQL 服务，第一阶段改用嵌入式
   SQLite（单文件、无需额外服务、便于备份）。`db/` 层隔离了存储细节，
   迁移 PostgreSQL 时不改领域模型与策略引擎。
2. **合约**：第一阶段只完成《执行合约详细设计 v1.0》的设计与接口/测试规范，
   未编写 Solidity 代码、未部署、未启用真实签名（符合"确认前不进入编码"的边界）。
   `/api/contract/status` 明确返回 `deployed: false`。
3. **历史数据**：未导入真实历史 L2 盘口与归档区块。`audit` 只审计本机已记录的 Paper
   数据，样本不足直接给 `WATCH`，不编造缺失区间。
4. **行情**：默认 `feed.mode = "demo"`，合成市场用于离线验收全链路并刻意制造价格错位；
   它比主网常态更"有机会"，**Paper 盈利不能推导为实盘收益**。
   实时公共行情适配器 `cex/binance.rs` 已实现（只读、无 API Key、无下单），
   但尚未作为默认扫描源。
5. **编译目录**：本机 DSH 会话对工作区路径附加文件系统过滤，`cl.exe` 在工作区内编译
   C 依赖会失败，因此 `backend/.cargo/config.toml` 把 target 指向 `D:/arbx-build/quant-arb`。
   迁移机器时可删除该配置恢复默认目录。
6. **未做的部分**：永续合约/杠杆、提币与跨链桥、多交易所实盘、V4/Hook 池、
   ClickHouse/Redis、逐笔人工审批（PRD 明确不做）。
7. **M2 / REVM 未接入**：本机只有 M1 状态一致性与 M3 参数化压力模拟，没有本地 EVM 分叉执行，
   因此不产生 VERIFIED；M2 请求显式返回 `UNAVAILABLE`。M3 不是链上竞争回放，
   不能替代历史审计。
8. **历史回放未实现**：未接入归档区块与区块内交易顺序重建，历史审计仍只统计本机已记录数据。
9. **容量与行情**：单笔容量按「单池一侧储备价值 × `max_pool_share_pct`」保守估算；
   默认仍是合成行情，真实 RPC/WebSocket Pool 发现与多链注册表尚未接入。
10. **版本控制**：仓库已初始化，远端为 <https://github.com/a190892593/quant-arb.git>；
    `.gitignore` 排除构建产物、本地数据库与前端依赖。

## 8. 测试

```
cargo test          # 212 个测试（202 lib + 10 集成）
```

- 领域层：金额精度、枚举契约、计划哈希、Decimal JSON 形状。
- DEX：V2/V3/StableSwap/Weighted 数学、跨 tick、池集合索引；准入、多重图、指纹、有界搜索、
  投入优化、Gas 分项、M1/M3 模拟、增量缓存。
- CEX：盘口序号缺口/乱序失效、逐档吃单、IOC/FOK/限价、精度与费率、Binance 解析。
- 风控：49 项参数、门槛计算、逐项破坏的拒绝 key、覆盖严格性、熔断窗口。
- 账本：期初分配、透支拒绝、批量原子性、费用账户、绩效与回撤、导出恢复。
- 集成：`tests/paper_flow.rs`（DEX 成功/回滚/资金不足/计划过期/盘口方向）、
  `tests/cex_dex_flow.rs`（双腿完成/部分成交重算/CEX 未成交不执行第二腿）、
  `tests/multihop_scan.rs`（合成市场端到端：指纹唯一、优化自洽、增量复用、落库重读）。

## 9. 安全声明

- 第一阶段没有实盘签名器、没有 CEX 下单凭据、没有提款权限。
- 所有"执行"都是本地模拟；`live_trading_enabled` 恒为 `false`。
- 风控硬限制由后端强制；前端开关不构成授权。
- 不能用合约最低回款检查替代 CEX–DEX 双腿风险检查（两者在代码中分离）。
- 扫描引擎不持有私钥、不广播交易；所有机会都能追溯到池、区块、报价与模拟记录。

## 10. DEX 多池多跳引擎（本次升级）

按 `doc/DEX 扫描引擎需求文档.md` 对原扫描功能做了整体替换：

- **准入前置**：Token（UNKNOWN/OBSERVING/APPROVED/RESTRICTED/REJECTED）与 Pool（验证、
  执行白名单、TVL、状态年龄、流动性）不通过就不进图。
- **多重图**：一个池两条有向边，禁止把不同池合并成一条报价边。
- **有界搜索**：2–5 跳（可配 8），池不复用、中间 Token 不重复；候选数 / 访问量 / 时间
  / 池数四重预算，命中即截断并记录。
- **指纹去重**：有向、无方向、池序列、Token 序列、起始资产五类指纹；镜像方向算独立机会。
- **投入优化**：配置档位 + 对数网格 + 黄金分割细化 + 整数精度复算；输出最小/最优/最大
  可盈利投入与曲线。
- **成本与 Gas**：按 PRD §5.5 口径只扣真实现金成本；Gas 分项（AMM/转账/Executor/
  闪电贷/Approve）。
- **模拟**：M1 同区块状态一致性 + M3 压力；M2 未接入 REVM 时显式 UNAVAILABLE，
  不宣称 VERIFIED。
- **增量扫描**：池 → 路线反向索引，只重算受影响的路线并复用其余报价。

API：`GET /api/routes`、`/api/routes/{id}`、`/api/dex/graph`、`/api/scanner/engine`、
`/api/simulations`、`POST /api/simulations`；PRD §15 的 `/api/v1/*` 别名同步可用。
前端入口：`#/routes`（路由搜索中心）。

配置见 `config/default.toml` 的 `[dex_engine]`；`enabled = false` 可回退旧版三角/跨池扫描。
完整设计、口径与限制见 [DEX 扫描引擎技术升级说明](doc/DEX扫描引擎技术升级说明.md)。