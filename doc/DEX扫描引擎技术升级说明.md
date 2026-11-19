# DEX 多池多跳扫描引擎 · 技术升级说明

**文档版本：** v1.0  
**对照需求：** `doc/DEX 扫描引擎需求文档.md`（PRD v1.0）  
**实现版本：** `arb-backend`（Rust + Axum，Local First / Paper）  
**升级范围：** 扫描引擎（准入、图、搜索、报价、优化、Gas、模拟、机会生命周期、增量扫描、API/DB/前端）

---

## 1. 升级目标与边界

本次升级把原来的「三角 + 跨池」两段式扫描替换为统一的 **多池多跳扫描引擎**：

- 搜索范围从「最多 4 跳的三角/跨池」扩展到 **2–8 跳可配置**（默认 2–5 跳，覆盖 PRD §4.4 初始参数）；
- 从「一次扫描一个金额档位」升级为 **逐路线投入金额优化**（档位 + 对数网格 + 黄金分割细化）；
- 从「模型等级写死 M2」改为 **按真实能力标注 M0/M1/M3**，并显式声明 M2（本地 EVM）不可用；
- 从「每轮全量重算」升级为 **池 → 路线反向索引 + 增量复用**；
- 新增 Token/Pool 准入、路线指纹、生命周期、路线级门禁，并把完整路线记录落库、通过 API 与前端展示。

**不在本次范围（与 PRD 非目标一致）：** CEX 行情/执行、跨链、桥、私钥与实盘签名、自动实盘交易；历史 L2 状态回放与 EVM 分叉模拟也未在本机接入（见 §9）。

---

## 2. PRD 章节映射

| PRD 章节 | 落地实现 | 说明 |
|---|---|---|
| §2 路线类型 | `RouteType`（CROSS_POOL / TRIANGLE / MULTI_HOP） | 按跳数归类；2 跳跨池、3 跳三角、4+ 多跳 |
| §3.3 Token 白名单 | `dex::registry`、`TokenStatus` | UNKNOWN/OBSERVING/APPROVED/RESTRICTED/REJECTED；只收紧不放宽 |
| §4.1/4.2 多重图 | `dex::multigraph::TokenGraph` | 一个池两条有向边，禁止合并抽象报价边 |
| §4.3 闭环规则 | `dex::route_search` | 同链、起点闭合、池不复用、中间 Token 不重复 |
| §4.4 跳数与预算 | `SearchLimits` + `[dex_engine]` 配置 | 最小/最大跳数、最大池数、候选数、访问量、时间预算 |
| §4.5 搜索算法 | 有界 DFS（分层：准入 → 图 → 搜索 → 报价 → 优化 → 模拟 → 排序） | 不暴力枚举全组合 |
| §4.6 路线去重 | `dex::fingerprint` | 有向/无方向/池序列/Token 序列/起始资产五类指纹 |
| §5.1–5.3 精确报价 | `dex::pool::quote_path` + 逐 AMM 模型 | 逐跳复用上一跳输出，整数/Decimal 计算 |
| §5.4 投入优化 | `dex::optimizer` | 档位 + 对数网格 + 黄金分割，返回最小/最优/最大可盈利投入 |
| §5.5 成本 | `strategy::dex_scan::price_amount` | 只扣除真实现金成本；AMM 手续费与滑点已内含在成交价（见 §5） |
| §5.6 Gas | `dex::gas` | AMM + 转账 + Executor + 闪电贷 + Approve 分项 |
| §6.1 模拟分级 | `dex::sim` | M0 理论、M1 状态一致、M2 显式 UNAVAILABLE、M3 压力 |
| §6.2 状态一致性 | M1 校验所有池同一区块；`DataQuality::Invalid` 阻止机会 | 不同区块禁止拼接 |
| §6.3 报价/模拟差异 | `SimulationOutcome.diff_bps`、`model_mismatch` 门禁 | 差异 > 阈值判 MODEL_MISMATCH |
| §7.1 生命周期 | `OpportunityLifecycle` | DISCOVERED → QUOTED → PROFITABLE_ESTIMATE → SIMULATED → VERIFIED；REJECTED/EXPIRED/FAILED |
| §7.2 利润门槛 | 复用 `RiskEngine::min_profit_for`（DEX：max(2 USD, 10 bps)） | 绝对金额与 bps 同时约束 |
| §7.4 机会记录 | `RouteScanRecord` + `Opportunity` | 指纹、逐腿、优化曲线、M1/M3、拒绝原因、区块与时间 |
| §8.1/8.2/8.3 准入与风险 | registry + 路线级门禁 + 风控引擎 | 任一池/Token 不可信即降级整条路线 |
| §9.2 增量更新 | `MultiHopCache` + 池 → 路线索引 | 池状态/价格/年龄变化才重算 |
| §12 风控规则 | 既有 49 项参数 + 新增路线门禁 | 硬限制在后端，前端不能放行 |
| §13.3 数据表 | `dex_routes`、`route_fingerprints`、`simulations`、`scan_runs` | 保存完整 JSON 与关键筛选列 |
| §14.3 路线搜索中心 | 前端 `#/routes`（RouteSearch 视图） | 见 §8 |
| §15 API | `/api/routes`、`/api/dex/graph`、`/api/simulations`、`/api/scanner/engine` + `/api/v1/*` | 兼容旧接口 |
| §18 设计原则 | 不重复扣费、不冒充 M2/VERIFIED、不混用区块、预算有界 | 见 §10 |

---

## 3. 模块与数据流

```
MarketView（同链池/Token/价格）
   │
   ├─ TokenRegistry::build            准入：Token 状态、风险标签、执行权限
   ├─ TokenGraph::build               多重图：仅通过准入的池贡献有向边
   ├─ route_search::search            有界 DFS：跳数/池数/候选/访问量/时间
   ├─ fingerprint::fingerprints       有向 + 无方向 + 池/Token 序列指纹
   ├─ optimizer::optimize             投入金额：档位 + 网格 + 细化
   ├─ price_amount                    逐跳报价 + Gas + 现金成本
   ├─ sim::simulate_m1                状态一致性（同区块、复算一致）
   ├─ sim::simulate_m3                压力（池状态/Gas/闪电贷费用扰动）
   ├─ gate_route                      准入、模拟、跳数、容量、状态质量
   └─ RouteScanRecord + Opportunity   生命周期 / 指纹 / 落库 / API / 前端
```

关键代码：

- `backend/src/dex/registry.rs`：Token / Pool 准入与原因链
- `backend/src/dex/multigraph.rs`：Token–Pool 有向多重图与稳定签名
- `backend/src/dex/route_search.rs`：预算约束 DFS、统计与截断
- `backend/src/dex/fingerprint.rs`：五类路线指纹
- `backend/src/dex/optimizer.rs`：投入金额优化
- `backend/src/dex/gas.rs`：Gas 分项与误差
- `backend/src/dex/sim.rs`：M0/M1/M2/M3
- `backend/src/strategy/dex_scan.rs`：编排、成本、门禁、机会与增量缓存

---

## 4. 搜索与去重

- **图**：每个池贡献 `token0→token1` 与 `token1→token0` 两条有向边；不同池永不合并。
- **闭环**：DFS 回到起始 Token 即闭合；闭合长度必须落在 `[min_hops, max_hops]`，池不重复、中间 Token 不重复。
- **预算**：`max_candidates`、`max_visits`、`max_elapsed_ms`、`max_pools`、`max_hops` 任一命中即停止并置 `truncated=true`，统计写入 `ScanRunStats`。
- **去重**：`directed` 指纹 = 池序列 + 方向；同一有向池序列只保留一次。镜像方向保留为独立机会（PRD §4.6 明确两个执行方向不同）。
- **确定性**：池按 `pool_id` 排序、出边按 `(pool_id, token_out)` 排序，同样输入产生同样候选顺序。

---

## 5. 成本口径（重要变化）

旧实现同时从毛利中扣除「DEX 手续费」和「价格冲击」，但 AMM 的 `amount_out` 本身已经包含手续费与滑点，属于重复扣减。新引擎按 PRD §5.5 的口径：

```
NetProfit = 最终回款价值 − 初始投入价值 − Gas − 闪电贷费用 − Builder/优先费 − 延迟/MEV 准备金 − 其他可计量成本
```

- DEX 手续费与价格冲击**只作为信息性指标**记录（`RouteScanRecord.embedded_fee_usd`、逐腿 `price_impact_bps`），不再重复扣减；
- 自有资金不计闪电贷费用；只有实际使用闪电贷时按配置费率计费；
- 成本明细仍逐项可加总，`Opportunity.total_cost` 与 `costs` 明细同源；
- 压力模拟会把 Gas 与闪电贷费用上调后再算净利。

该变化会让 DEX 机会的净利高于旧实现；这是口径修正，不是收益改善。

---

## 6. 模拟分级与诚实边界

| 等级 | 实现 | 用途 |
|---|---|---|
| M0 | 即期价连乘（`theoretical_out`） | 仅作对比，不生成机会 |
| M1 | 同区块快照逐跳复算 + 与报价输出比对 | 所有可执行机会必须通过 |
| M2 | `simulate_m2_unavailable` | 本机未接入 REVM，显式 UNAVAILABLE |
| M3 | 池储备/价格不利扰动、Gas 与闪电贷费用上调后重算 | 抗压判断，负值只触发 WARNING |

因此 **生命周期最高到 SIMULATED**；`VERIFIED` 保留给未来接入 REVM 的 M2 结果。前端与 API 不得把 M1/M3 说成「已验证」。

---

## 7. 增量扫描

- 缓存保存：路线集合、池 → 路线反向索引、图签名、池区块号、每条路线上次报价时间与价格签名。
- 触发全量：图签名变化（池/费率/方向/准入变化）、首次扫描、每 `full_rescan_blocks` 个区块。
- 触发单路线重算：所涉任一池区块变化、价格签名变化、报价年龄超过 `requote_age_ms`、从未报价。
- 未受影响的路线复用上次记录（`source = "incremental"`），只更新年龄；超过 TTL 标记 EXPIRED。
- 统计字段 `requoted` / `reused` / `visits` / `elapsed_ms` 通过 API 与前端展示，便于观察命中率。

---

## 8. API、数据库与前端

### API（新增，旧 `/api/*` 保持不变）

```
GET  /api/routes?route_type=&lifecycle=&start_token=&min_net=&min_hops=&max_hops=&limit=
GET  /api/routes/{route_id}
GET  /api/dex/graph
GET  /api/scanner/engine
GET  /api/scanner/config
POST /api/scanner/restart
GET  /api/scanner/runs
GET  /api/simulations
POST /api/simulations            { "route_id": "...", "level": "M1" | "M3" | "M2" }
GET  /api/simulations/{simulation_id}
```

PRD §15 的 `/api/v1/*` 别名复用同一批处理器（system/chains/protocols/tokens/pools/scanner/routes/opportunities/simulations/audits/paper/risk）。

### 数据库（新增表）

- `dex_routes`：路线快照与关键筛选列；
- `route_fingerprints`：五类指纹；
- `simulations`：M1/M3/M2 结果；
- `scan_runs`：每轮扫描统计（候选、重算、复用、访问量、截断）。

金额与数量继续以 TEXT 保存 Decimal 原文。旧库通过幂等 `CREATE TABLE IF NOT EXISTS` 自动升级。

### 前端

新增「路由搜索中心」（`#/routes`）：图层统计、路线表、筛选、逐腿详情、指纹、投入曲线、M1/M3 结果与拒绝原因。数据来自 `/api/routes` 轮询，后端不可达时明确降级，不伪造数据。

---

## 9. 验证

```
cd backend
cargo test            # 全部单元 + 集成测试
cargo build           # 产出 arb-backend
```

关键测试：

- `dex::registry::tests`：准入只收紧、池拒绝原因；
- `dex::multigraph::tests`：每池两方向、不可信池/Token 不贡献边、签名变化；
- `dex::fingerprint::tests`：镜像方向共享无方向指纹、有向指纹不同；
- `dex::route_search::tests`：2/3/4 跳、候选/访问量截断、确定性、池不复用；
- `dex::optimizer::tests`：解析最优附近收敛、边界、量化、非单峰说明；
- `dex::gas::tests`：分项加总、闪电贷/Approve、误差符号；
- `dex::sim::tests`：M1 通过/混合区块/模型不一致/重复池，M2 UNAVAILABLE，M3 输出下降且不污染原状态；
- `strategy::dex_scan::tests`：端到端路线与记录、增量复用、容量上限、未准入 Token 不进入路线；
- `tests/multihop_scan.rs`：合成市场端到端、指纹唯一、优化自洽、增量复用、落库重读。

运行验证：启动后端后 `GET /api/routes` 返回 46 条候选路线（8 池 / 16 边 / 4 个图内 Token，默认配置），M1 PASSED、M3 压力结果与拒绝原因完整；`POST /api/simulations` 的 M1/M3/M2 三种请求分别返回 PASSED / 压力结果 / UNAVAILABLE。

---

## 10. 限制与后续

1. **M2/REVM 未接入**：没有本地 EVM 分叉执行，无法产生 VERIFIED；接入 REVM 后应把 M2 结果写入 `simulations`，并把 VERIFIED 限定为 M2 通过。
2. **M3 是参数化压力**：扰动池储备/价格与 Gas，不是真实链上竞争回放；不能替代历史审计。
3. **历史回放未做**：不存在区块内交易顺序重建、缺失事件标记与 30/90/180/365 天窗口统计。
4. **行情仍是合成市场**：默认 `feed.mode = "demo"`；真实 RPC/WebSocket Pool 发现、事件解码与多链注册表尚未接入。接入时 `PoolSet` 与 `PoolState` 已是边界。
5. **容量是保守近似**：按单池一侧储备价值 × `max_pool_share_pct` 估算，未做逐 tick/tick 位图的精确容量曲线。
6. **未使用 Redis/ClickHouse/Prometheus**：本地单进程 SQLite 起步；表结构已与领域模型分离，后续可替换。
7. **前端仅实现路线搜索中心**：扫描控制台（跳数/预算/档位在线修改）仍由配置文件 + 重启完成，PRD §14.2 的完整在线控制未实现。