# DEX 原子套利研究与 Paper 交易系统

## 总体技术设计说明书（TDD v2.0）

| 项目 | 内容 |
|---|---|
| 文档版本 | v2.0 |
| 基线版本 | TDD v1.2 |
| 适用代码 | `backend/`（Rust）、`frontend/`（React） |
| 更新日期 | 2026-11-20 |
| 修订 | 0720352 移除 CEX 扫描；ef33774 标价链上主源 + 净利双口径；cece2f1 移除 CEX–DEX 非原子策略 |
| 运行形态 | 本地双实例（Arbitrum 8791 / Base 8792） |
| 交易阶段 | Research + Paper，Live 关闭 |

> 本设计只描述已落地或明确标注为“未实现”的代码路径。所有安全关键能力以当前仓库代码为准。

---

## 1. 技术目标与原则

### 1.1 目标

1. **可复现**：同一区块状态、同一配置必须得到同一报价与机会。
2. **可解释**：每条路线可下钻到跳、池、费率、容量、Gas、逐档净利与拒绝原因。
3. **可降级**：M2/RPC 失败时不阻塞 M1 扫描，证据明确回退。
4. **不伪造**：缺失价格、错误池方向、错误精度必须被检查捕获。
5. **Paper 优先**：所有执行链路只做模拟，不接触私钥与广播。

### 1.2 原则

- 配置驱动：链、Token、池、数据源、搜索参数都在 TOML/JSON；
- 单写者：SQLite 单进程写，读多写少；
- 只读链上：`eth_call`/`eth_getStorageAt`/`eth_getCode`，不发交易；
- 金额用 `Decimal`，链上原始单位用 `u128/i128/U256`；
- 所有报价绑定 `(block_number, updated_at)`。

---

## 2. 技术栈

| 层 | 技术 |
|---|---|
| 后端 | Rust 1.100，edition 2024 |
| Web | Axum 0.8.9，tower-http 0.7 |
| 异步 | Tokio 1.54 |
| 数据库 | SQLite（rusqlite 0.43，bundled，WAL） |
| EVM | revm 43.0.3，ForkDb + state override |
| 数值 | rust_decimal 1.43 |
| 序列化 | serde/serde_json/toml |
| 哈希 | tiny-keccak |
| 前端 | React 19.1、TypeScript 5.9 strict、Vite 7.1 |
| 图表 | 手写 SVG |
| 样式 | `frontend/src/styles/app.css` |

---

## 3. 部署架构

### 3.1 进程模型

```
Arbitrum 实例 (8791)                Base 实例 (8792)
┌─────────────────────┐            ┌─────────────────────┐
│ config/default.toml │            │ config/default.toml │
│ + 无 profile        │            │ + config/base.toml  │
│ workspace: data/    │            │ workspace: instances/base/data/ │
└─────────┬───────────┘            └─────────┬───────────┘
          │                                  │
          └────────── UI (ui/) ──────────────┘
                  同一份静态构建
```

- `ARBX_PROFILE=base` 触发 TOML 深合并；
- `ARBX_WORKSPACE` 决定数据目录；
- `ARBX_PORT` 覆盖端口；
- `ARBX_UI_DIR` 指向 UI 构建目录；
- 两实例独立数据库、独立 feed、独立扫描器。

### 3.2 启动流程

1. 加载配置（default + profile overlay）；
2. 初始化 SQLite、迁移、恢复账本快照；
3. 构建 `LiveFeed`（Token/池发现与预热）；
4. 构建 `DexScanParams`/`SearchConfig`；
5. 启动 Axum HTTP/WS；
6. 启动 feed 循环、扫描循环、发现任务（可选）。

---

## 4. 代码结构

```
backend/src/
  main.rs / lib.rs
  config.rs                配置与 profile 合并
  engine/
    mod.rs                 AppState、路由、后台任务
    scanner.rs             扫描编排、增量、持久化、M2 触发
    seed.rs                初始链/协议/Token 种子
  feed/
    live.rs                链上/CEX 行情、池状态、tick、稳定池
    demo.rs                仅测试用
  dex/
    discovery.rs           聚合器发现、TVL/协议过滤、自动登记
    registry.rs            Token/池注册与准入
    multigraph.rs          TokenGraph、边、统计
    route_search.rs        DFS 简单环
    pool.rs                PoolRuntime、quote、capacity
    math.rs                V2/V3/Stable/Weighted 数学
    optimizer.rs           投入规模网格/细化
    sim.rs                 M1/M3
    gas.rs                 Gas 估算
    fingerprint.rs         路线指纹
  price/
    oracle.rs              标价三源合并（链上 spot / Chainlink / CEX）+ 分歧
  strategy/
    dex_scan.rs            路线缓存、逐档、机会生成
    m2.rs                  QuoterV2 + M2-TX 原子回放
    cost.rs / scorer.rs    成本与评分
    （cex_dex.rs 于 0720352 删除；paper 的 execute_cex_dex / simulate_cex_leg / unwind_cex 于 cece2f1 删除）
  evm/
    rpc.rs                 多端点、批量、estimateGas
    fork.rs                ForkDb、代码/存储覆盖
    revm_sim.rs            REVM 执行
    atomic_sim.rs          闪电池 executor 回放/估算
    abi.rs                 编解码
    executor.hex           executor runtime bytecode
  risk/ ledger/ paper/ db/ api/ domain/
frontend/src/
  views/ components/ data/ lib/ styles/
```

---

## 5. 配置体系

### 5.1 文件与优先级

1. `config/default.toml`；
2. `config/<profile>.toml`（深合并，数组替换）；
3. 环境变量：`ARBX_PROFILE`、`ARBX_WORKSPACE`、`ARBX_PORT`、`ARBX_UI_DIR`；
4. 运行时可写配置：
   - `data/source_config.json`
   - `data/search_config.json`
   - `data/discovered_pools.json`
   - `data/discovery_state.json`

### 5.2 关键配置

| 配置 | 说明 |
|---|---|
| `[server]` | host/port/ui_dir/cors |
| `[data]` | dir/db_file/保留条数 |
| `[paper]` | 期初权益、分配、扫描周期、TTL、档位、并发 |
| `[cex]` | Binance 符号、费率、盘口档数（仅定价/数据质量，不产生 CEX–DEX 机会） |
| `[dex]` | Gas 兜底、闪贷费、每跳 Gas |
| `[dex_engine]` | 跳数、候选/预算、TVL/年龄/容量、M3、增量 |
| `[live]` | chain_id、RPC 主/备/池、刷新、池上限、tick、Token、额外池 |
| `[m2]` | Quoter、tolerance、gas limit |
| `[discovery]` | 开关、网络、Key、TVL、池数 |
| `[sources]` | 数据源 JSON 路径/策略 |

### 5.3 RPC 策略

- `RpcClient` 持有端点列表与当前节点；
- HTTP 错误、JSON-RPC 错误、响应解码失败都会切换端点；
- `batch_eth_call` 按 40 个请求分块，2 组并发；
- 批量失败时回退单请求；
- 端点池当前以 publicnode 为主，drpc/arb1 视可用性配置；
- 公共 RPC 的 429/批大小限制是已知运行风险。

---

## 6. 数据层

### 6.1 SQLite

WAL 模式，单写者；核心表：

| 表 | 用途 |
|---|---|
| `schema_migrations` | 迁移 |
| `chains` / `dex_protocols` | 链/协议 |
| `tokens` / `pools` / `pool_states` | Token/池/快照 |
| `cex_exchanges` / `cex_instruments` / `cex_data_health` | CEX |
| `strategies` | 策略定义 |
| `opportunities` | 机会 |
| `trade_plans` | 交易计划 |
| `executions` / `cex_fills` / `execution_events` / `unwind_events` | 执行状态机 |
| `accounts` / `ledger_entries` / `inventory_snapshots` | 账本/库存 |
| `risk_events` / `risk_params` / `risk_overrides` / `system_controls` | 风控 |
| `system_configs` | 配置快照 |
| `audit_jobs` / `audit_reports` | 审计 |
| `scanner_state` / `dex_routes` | 扫描缓存/路线 |

### 6.2 JSON 文件

- `data/source_config.json`：27 数据源、Key、策略；
- `data/search_config.json`：Token 准入与搜索参数；
- `data/discovered_pools.json`：发现池与价格；
- `data/discovery_state.json`：发现游标/时间。

### 6.3 数据保留

- 机会/风险事件按配置截断；
- 审计/账本不自动删除；
- Paper 快照可从 DB/账本恢复。

---

## 7. 链数据 Feed

### 7.1 Token/池加载

- 配置的 Token 先注册，启动时链上校验 decimals；
- 额外池按配置解析；
- 自动发现池合并，未知协议跳过；
- 池预筛：TVL、地址重复、协议白名单；
- 池状态批量读取：
  - V2/Stable：`getReserves()`；
  - V3：`slot0()` + `liquidity()` + Token `balanceOf(pool)`；
  - 费率缺省时用注册表补齐；
  - 空数据/合约不存在累计后剔除。

### 7.2 V3 Tick 加载

- 配置：`tick_word_radius`、`max_ticks_per_pool`、`tick_top_pools`、`tick_refresh_ms`；
- 仅对 TVL 前 N 的 V3 池；
- 由当前 tick 计算 word position，读取 `tickBitmap(int16)`；
- 解析 bitmap 的置位 bit，读取 `ticks(int24)`；
- 写入 `PoolState.ticks`，按 tick 排序并截断；
- 缓存 60 秒；
- 当前状态：
  - Base 开启（radius=1，top=8），实测 5 池加载 16/3/3/3/3；
  - Arbitrum 关闭（radius=0），公共 RPC 下开启会超时。

### 7.3 稳定池

- `PoolKind::StableSwap`；
- Aerodrome/Velodrome 稳定池无 A 参数，使用固定不变式：

```
k(x,y) = x³y + y³x
```

- 数学实现：`math::aerodrome_stable_amount_out`（牛顿迭代，先扣费）；
- spot 使用曲线 1 单位边际价，不使用 `reserve1/reserve0`；
- 已验证与 Base WETH/USDC 稳定池 `getAmountOut` 一致（1 WETH → 1326.394680 USDC，0 bps）。

### 7.4 价格（标价 / mark price）

标价只用于**美元换算**（名义金额、池 TVL 门槛、容量上限、Gas 美元成本），
**不参与 AMM 定价** —— 成交输出由链上池状态经 `quote_path` 精确计算，因此标价
误差不会制造价差，只按比例影响美元口径的数值。

三源分层（`backend/src/price/oracle.rs`）：

| 层 | 来源 | 用途 |
|---|---|---|
| L1 主源 | **链上最深计价池 spot 价**（`PoolRuntime::spot_price`，与 USDC/USDT 配对、按 TVL 取最深） | 决定最终标价 |
| L2 校验 | **Chainlink 聚合器**（Arbitrum：ETH/USD `0x639Fe6…a612`、BTC/USD `0x6ce185…08e9`、LINK/USD、ARB/USD、USDC/USD），读 `latestRoundData()`，校验 `updatedAt` 新鲜度（> 24h 丢弃） | 抗单池瞬时操纵 |
| L3 兜底 | Binance 中间价（仅第一、二层都缺该 Token 时生效） | 缺失补齐 |

合并规则：

- USDC / USDT / DAI 固定 1 美元，不参与合并；
- 其余 Token 取三源**中位数**为最终标价；
- 记录每个 Token 与中位数的**最大偏差（bps）**，输出到 `/api/system/prices`；
- 偏差 > 50 bps 标记为分歧，相关路线在扫描阶段降级为「观察」（见 §12.3）。
- 白名单外 Token（发现源带入的未登记/乱码符号）在合并结果与推导价中一律剔除。

当前实测：WETH 分歧 12–13 bps、WBTC 8 bps、其余 0 bps，最大 13 bps。

### 7.5 健康

- 记录 feed 延迟、区块年龄、池失败数、最后错误；
- feed 超时保留上一份快照并标记不健康；
- API `/system/health`、`/markets/data-health` 可观测。

---

## 8. DEX 发现

- 来源：GeckoTerminal、CoinGecko、DexScreener；
- 网络重写：`/arbitrum/` → `/{network}/`；
- 候选字段：地址、协议、Token0/1、TVL、价格、来源；
- 过滤：
  - 协议未知 → 跳过；
  - TVL < 1 万或 > 10 亿 → 跳过；
  - 地址重复 → 跳过；
- 自动登记 Token：
  - 新 Token 写入白名单/待审；
  - 可在 UI 修改准入；
- 周期默认 1,800 秒，4 页，避免免费额度耗尽；
- 结果持久化，重启恢复。

---

## 9. 图与路线搜索

### 9.1 图

- 节点：链内 Token（当前以 symbol 为键，链内唯一）；
- 边：池方向（token_in → token_out），带 pool_id/protocol/kind/fee/TVL；
- 仅 `admission.eligible` 的 Token/池入图；
- 图统计：tokens、pools、edges、eligible/rejected。

> 当前未实现跨链统一图；每实例独立图。路线 ID 已带链前缀：`route_{chain_id}_{fingerprint}`。

### 9.2 DFS

```text
for start in base_symbols:
  dfs(start, current=start, depth=0, used_pools=[], used_tokens=[start])
    if depth >= max_hops: return
    for edge in edges(current):
      if edge.pool in used_pools: skip
      if edge.token_out in used_tokens and edge.token_out != start: skip
      if edge.token_out == start:
        emit cycle (2..max_hops hops, distinct pools)
      else:
        recurse
```

- 预算：max_candidates、max_visits、max_elapsed_ms；
- 超限标记 truncated；
- 指纹：直接/无向/pool/token 序列，用于去重与缓存。

### 9.3 缓存与增量

- `CachedRoute`：route_id、edges、fingerprints、last_quote_at、price_sig、record；
- 重报价条件：全量、池变化、过期、价格签名变化、无记录；
- `requote_age_ms` 默认 4 秒；
- 全量重建按 `full_rescan_blocks` 或手动。

---

## 10. 定价模型

### 10.1 V2

```
out = amount_in * (1 - fee) * reserve_out / (reserve_in + amount_in * (1 - fee))
```

### 10.2 V3

- 单区间精确公式：`v3_amount_out_single_range`；
- 跨 tick：`v3_quote_with_ticks`，逐 tick 扣减流动性，最多 200 tick；
- 输入金额先按 Token 精度转 raw，再回人类单位；
- `sqrtPriceX96` 与 `liquidity` 必须非零，否则 `ZeroLiquidity`。

### 10.3 Stable

- Aerodrome：固定 x³y+y³x（无 A）；
- Curve 类：`stableswap_amount_out`（A 参数）；
- 当前 Aerodrome 池 `amplification=0`，走固定曲线。

### 10.4 报价路径

- `quote_path` 逐跳调用 `quote_exact_in`；
- 记录每跳 amount_in/out、fee、impact、gas_units、block；
- 复用池检查（同一池不可重复）；
- 输出 `PathQuote`。

### 10.5 容量

```
capacity = min_i(reserve_in_i * price_in_i * max_pool_share_pct)
```

- 容量为 0 → 路线不报价；
- 最大投入 = min(capacity, max_input_usd)；
- 最小投入 = min_input_usd / start_price。

### 10.6 规模优化与逐档

- 网格 `grid_points` + 细化 `refine_iters` 找最优投入；
- 固定 ladder 再逐档报价（不插值）：
  - `SizeQuote { notional_usd, amount_in, amount_out, gross_usd, total_cost_usd, net_usd, net_bps }`；
- 逐档成本与 Gas 同口径；
- 档位受容量裁剪，超出容量的档位不输出。

---

## 11. 成本与 Gas

### 11.1 Gas 组成

```
gas_units = executor_base + per_hop * hops + flash_loan + approvals
gas_usd = gas_units * gas_price_native * native_price_usd
```

- 运行时优先用链上 `eth_gasPrice`/basefee 与 WETH 价格；
- 配置兜底：`gas_price_gwei`、`native_price_usd`、`gas_per_hop`；
- 优先级费用、延迟储备、MEV 储备按 bps 计入。

### 11.2 闪贷费

- `flash_loan_fee_bps` 必须来自配置/协议；
- 默认 5 bps，禁止隐式 0；
- M2-TX 使用真实 V3 池 flash fee（pool.fee）。

### 11.3 净利（双口径）

```
net_token = amount_out − amount_in          # 起始资产口径，纯链上，无标价污染
net_usd   = gross − gas − priority − flash − delay − mev
gross     = (amount_out − amount_in) × start_price
```

设计意图：把「价差够不够」与「成本重不重」拆开。

- 起始资产为稳定币时 `start_price = 1`，`net_usd` 与 `net_token` 的差额即全部执行成本；
- 路线表同时展示两个口径（USD 主行 + Token 副行），据此判断亏损来源；
- 判定可执行仍以 `net_usd` 为准（成本必须以美元计价）。

> 当前实测样例：`net_usd = −0.16`、`net_token = −0.0954 USDC` → 价差缺口 0.095、成本 0.065，
> 说明当前市场**没有正收益机会**，不是成本吃掉了利润。

---

## 12. 机会、风险与生命周期

### 12.1 Opportunity

- 字段：id、strategy_type、chain_id/name、DEX/CEX legs、target_notional/qty、库存需求、成本、净利、bps、评分、风险、模型等级、状态、时间；
- `opportunity_id` 带链/路线/档位/时间信息（实现为哈希/拼接，保证链内唯一）。

### 12.2 生命周期

`DISCOVERED → QUOTED → RISK_CHECKED → ACTIONABLE/WATCH/REJECTED → EXPIRED/EXECUTED`

- TTL 30 秒；
- 拒绝原因码记录在 `rejections`；
- 过期/暂停后不可执行。

### 12.3 风险门禁

- 数据年龄、池 TVL/容量、评分、模型偏差、库存、滑点、日亏损、连续亏损、全局暂停；
- **标价分歧门禁**：路线涉及的任一 Token 三源标价偏差 > 50 bps 时，追加
  `price_divergence`（`SoftBlock`）拒绝项，机会降级为 `WATCH`，不做可执行判定；
- 断路器状态持久化；
- 参数可覆盖，但必须审计。

---

## 13. M1 / M2 / M2-TX / M3

### 13.1 M1

- 本地模型复算整条路径；
- 校验池状态同块、路径可执行、Token 准入；
- 输出 `SimulationOutcome`（M1）。

### 13.2 M2 QuoterV2

- 仅全 V3 路线；
- 构造 `quoteExactInput(bytes,uint256)`；
- REVM fork 在报价区块执行，比较 M1/M2 偏差；
- 偏差 ≤ tolerance → 通过；
- Quoter 回滚/超时 → 保留 M1，记录证据；
- Base 早期 Quoter 回滚，当前按路线/链配置启用。

### 13.3 M2-TX 原子回放

**合约**：`ArbExecutor`（Solidity 0.8.20，viaIR，runtime 约 2781 bytes，solc 0.8.39 编译），源码在构建目录，runtime bytecode 在 `backend/src/evm/executor.hex`。

**接口**：

```solidity
execute(
  address pool, address tokenIn, uint256 amount, address recipient,
  address[5] targets, address[5] tokens, uint256[5] amounts,
  uint256[5] lengths, bytes blob, uint8 n
) returns (uint256)

uniswapV3FlashCallback(uint256 fee0, uint256 fee1, bytes data)
```

**流程**：

1. 从 V3 池 `flash` 借出 tokenIn；
2. 回调内按 blob 切片，逐跳 approve + router call；
3. 校验 `balance >= amount + fee0 + fee1`；
4. 归还池子，剩余利润转给 recipient；
5. 任一跳失败整笔 revert。

**回放**：

- `ForkDb` 支持 `set_code` / `set_storage`；
- `call_on_fork_with` 注入 executor 代码与存储；
- calldata 编码 V3 `exactInputSingle` / V2 `swapExactTokensForTokens`；
- 最多 5 跳；
- 仅模拟，不写链。

**闪电池映射（当前）**：

| 链 | 起始 Token | 闪电池 |
|---|---|---|
| 42161 | USDC/USDT | USDC/USDT 1bps |
| 42161 | WETH | WETH/USDC 5bps |
| 42161 | DAI | DAI/USDC |
| 8453 | USDC/USDbC | USDbC/USDC 1bps |
| 8453 | WETH | WETH/USDC 5bps |
| 8453 | DAI | DAI/USDC 1bps |
| 8453 | AERO | USDC/AERO volatile |

**限制**：

- 路由 hop 仅支持 `UNISWAP_V3` / `UNISWAP_V2`；
- Aerodrome 专用 Router 未适配；
- 闪电池不能与路线内池相同（同池 swap 会 `LOK`）；
- 无正收益路线时会在还款校验处 `not enough`，这是正确行为。

### 13.4 `eth_estimateGas`

- 公共 RPC 支持 `eth_estimateGas` 第三参数 state override；
- `RpcClient::estimate_gas_with_code` 注入 executor code；
- `atomic_sim::estimate_route_gas` 复用整笔 calldata；
- 成功时 `M2Evidence.gas_source="eth_estimateGas"`；
- 失败回退 `revm`；
- 当前无正收益路线，尚无成功 gas 样本。

### 13.5 M3

- 压力：费率上浮、Gas 上浮、闪贷费上浮；
- 输出压力后净利/是否通过；
- 不改变真实报价，仅做风险展示。

---

## 14. CEX 公共行情作为定价输入（CEX–DEX 扫描已移除）

- commit 0720352 删除 `strategy/cex_dex.rs`、`StrategyEngine::scan_cex_dex` 与 `CexDexParams`；
- `scanner` 不再调用 CEX–DEX 扫描，旧引擎回退路径也只产出 DEX 原子机会；
- Binance 公共 REST 仍保留，但**降为标价第三票**：
  - `bookTicker`/`depth` 构建盘口；
  - 中间价、深度加权价、滑点；
  - 仅在链上 spot 与 Chainlink 都缺该 Token 时参与标价（见 §7.4）；
  - 盘口深度仍用于数据质量对比与历史 Paper 回归展示；
- `/api/opportunities` 只返回 `dex_triple` / `dex_cross`；
- CEX–DEX Paper 执行链路（双腿模拟、部分成交回滚、对账窗口）已随策略删除（cece2f1）；`/api/paper/hedges` 保留但恒返回空；
- 前端 `#/scanner-cex` 重定向到 `#/scanner-dex`，能力矩阵标记为 `off`。

---

## 15. Paper、账本与风控

- 账户：DEX/CEX/现金/费用（CEX 账户保留以兼容历史账本）；
- 执行链路：**只保留 DEX 原子执行**（`execute_dex_atomic`），CEX 双腿路径已删除；
- 复式记账：每笔执行成对分录；
- 库存快照按执行更新；
- 绩效：权益、回撤、胜率、盈亏比；
- 风控：规则目录（33 项参数、6 组）、覆盖、暂停/恢复、事件；§4.2 CEX–DEX 对冲与 §7 双腿对冲执行两组参数已随策略移除；
- 审计：任务/报告/数据质量/模型等级。

---

## 16. API 设计

### 16.1 系统

`GET /api/system/status|health|config|controls|database|restart-check`
`GET /api/system/prices` —— 逐 Token 标价（`symbol`/`usd`/`divergence_bps`）+ 主源说明 + 最大分歧；数据质量页与扫描页消费。

### 16.2 链/DEX

`GET /api/chains`、`/chains/{id}/health`、`/dex/protocols`、`/dex/pools`、`/tokens`、`/adapters`、`/dex/graph`

`/dex/pools` 关键字段：pool_id/address/protocol/kind/chain_id/token0/1/fee/tvl/verified/execution_allowed/reserve0/1/ticks_loaded/block/age/spot_price。

### 16.3 扫描/路线

`POST /api/scanner/start|stop|scan|restart`、`GET /api/scanner/status|config|engine|runs`

`GET /api/routes?limit=...`、`GET /api/routes/{id}`

`GET /api/opportunities`、`/opportunities/{id}`、`/simulations/{id}`

### 16.4 配置

`GET/PUT /api/search/config`、`POST /api/search/tokens`

`GET/PUT /api/sources`、`POST /api/sources/preset`

`GET /api/discovery/status`、`POST /api/discovery/scan`

### 16.5 CEX 定价数据（非机会来源）

`GET /api/exchanges`、`/exchanges/{x}/instruments`、`/markets/orderbook|prices|data-health`

说明：这些接口仍服务于 DEX 定价与数据质量；不再产出 CEX–DEX 机会。`/api/opportunities` 仅含 DEX 类型。

### 16.6 Paper/执行/风险/账本

`/api/paper/*`、`/api/executions*`、`/api/risk/*`、`/api/inventory`、`/api/ledger`、`/api/audit/*`、`/api/contract/status`

### 16.7 WebSocket

`GET /ws`：状态/机会/健康推送。

---

## 17. 前端设计

- React 19 + TS strict；hash 路由；无图表库（SVG 手写）；
- 导航分组：监控 MONITOR / 搜索 SEARCH / 交易 PAPER / 策略 STRATEGY / 研究 RESEARCH / 治理 GOVERN；
- 页面：Overview、RouteSearch（DEX–DEX 搜索引擎）、DataSources、Pools、Executions、Accounts、Performance、Strategy、Audit、DataQuality、Ledger、Risk、Contract、System、Capability；
- `#/scanner-cex` 旧链接重定向到 `#/scanner-dex`；导航不再提供 CEX–DEX 策略入口；

### 17.1 DEX–DEX 搜索引擎页（原「路线搜索」与「DEX 原子扫描」合并）

- 页面自上而下：页头（LIVE/刷新/搜索配置/启停）→ 扫描活动条 → 能力说明 → 搜索配置抽屉入口 → 引擎指标 → 路线/机会合表 → 抽屉；
- **路线表与机会流合为一张表**：以路线为主键（`route_id`），按 `opportunity_id` 关联机会对象补齐风险分、数据质量、处置状态；
- 列：类型 / 出现 / Token 闭环路径 / Pool 路径（单行截断 + 悬停全文）/ 跳数 / 投入 / 回款 / 毛利 / 显式成本 / 净利（USD + 起始资产双口径）/ bps / Gas / 压力净利 / 风险 / 数据 / 状态 / M1·M3 / 来源；
- 筛选：类型、净利阈值、跳数、状态；排序：多条件（最多 3 级，表头与排序栈双向同步），默认「出现时间 ↓」；
- 详情抽屉：逐腿、指纹、优化曲线、M1/M3 结果（按需请求 `/api/routes/{id}`）。

### 17.2 通用抽屉组件（`src/ui/Drawer.tsx`）

- 默认宽度 = 屏幕一半，左缘可拖拽调宽（380px ~ 94vw），双击复位；
- 宽度按 `storageKey` 记忆（localStorage）：搜索配置 / 路线详情 / 机会详情各自独立；
- 关闭按钮统一为 40×40 红色圆角按钮；支持 Esc 与遮罩关闭；
- 三个抽屉（搜索配置、路线详情、机会详情）全部复用该组件。

### 17.3 搜索配置抽屉

- 页头按钮打开右侧抽屉，替代此前的内联配置面板；
- 六组参数 + 投入阶梯 + Token 白/黑名单（见 PRD §5.4）；
- 保存即时生效，写入 `data/search_config.json` 与运行时 `DexScanParams`。

### 17.4 标价可见性

- 数据质量页顶部新增「标价源」面板：主源说明、逐 Token USD 标价与三源分歧、超阈值告警；
- 路线表净利列同时给出 Token 口径副行。

### 17.5 工程约定

- API 映射在 `src/data/api.ts`，类型在 `src/types.ts`；
- 构建：`npm run publish` → `ui/`；
- 同一 UI 由两个实例共用。

---

## 18. 并发、性能与预算

- feed 周期默认 2–2.5 秒；
- 扫描周期 1.5 秒；
- 单轮定价预算 `max_evaluate_ms=1200`、`max_evaluate=800`；
- 候选/访问上限防止组合爆炸；
- RPC batch 40；
- tick 仅 top N，缓存 60 秒；
- M2 异步触发，不阻塞 M1。

---

## 19. 可观测性与恢复

- 结构化日志（tracing）；
- `/system/health`：feed 延迟、区块年龄、池数、错误；
- `/markets/data-health`：价格/盘口年龄；
- 扫描统计：候选、重报价、盈利、拒绝、截断；
- SQLite 快照/账本恢复；
- Paper 重置；
- 扫描器可停/启/强制全量。

---

## 20. 测试体系

### 20.1 当前结果

- `cargo test`：**206 lib + 11 集成 = 217 通过**；
- 集成：`multihop_scan` 3、`paper_flow` 5（`cex_dex_flow` 随策略删除）；
- 前端 `tsc` 通过、`npm run publish` 成功。

### 20.2 关键测试

- 多跳扫描：闭环、池去重、跳数、逐档报价；
- 稳定曲线：单调、手续费、不均衡、链上样本；
- V3 跨 tick：单调、跨 tick、流动性耗尽上界；
- Paper：开户、成交、对冲、账本、重置；
- ~~CEX–DEX Paper 状态机~~：随策略移除（cece2f1），用例与集成测试一并删除；
- 风控：规则、熔断、覆盖。

### 20.3 手工/探针验证

- `examples/stable_probe.rs`：稳定曲线 vs 链上 `getAmountOut`；
- `examples/estimate_probe.rs`：`eth_estimateGas` + state override；
- `examples/atomic_probe.rs`：闪电池 executor 回放；
- `examples/balance_probe.rs`：fork 余额注入调试。

---

## 21. 安全与密钥

- 不加载私钥、不签名、不广播；
- 数据源 Key 仅本地 JSON，UI 掩码；
- 日志不打印完整 Key；
- executor 仅模拟，未审计、未部署；
- Live 阶段需另行设计密钥管理、合约审计、限额与合规。

---

## 22. 已知技术债与后续

| 项 | 说明 |
|---|---|
| 多链统一扫描 | 当前双进程；需共享图/缓存/机会聚合 |
| 地址级节点身份 | 当前链内 symbol 图 + 链前缀路线 ID；未全量改 `(chain,address)` |
| Aerodrome Router | 原子回放未适配专用 Router |
| Arb tick | 公共 RPC 超时，默认关闭 |
| eth_estimateGas 成功样本 | 无正收益路线，只有链路验证 |
| 公共 RPC | 429/批限制/连接重置，需专用节点 |
| 稳定池路线 | 曲线正确，但价格偏离，未进入闭环 |
| 6–8 跳 / BSC | 明确不做 |
| CEX–DEX 策略 | 已整体移除（0720352 移除扫描、cece2f1 移除枚举/执行/风控参数组/用例）；CEX 仅保留行情数据作为标价第三票与展示 |
| 标价分歧阈值 | 固定 50 bps 硬编码；后续改为可配置并接入风控参数表 |
| Chainlink 覆盖 | 仅 ETH/BTC/LINK/ARB/USDC 五个 feed；其余 Token 只有链上 spot 与 CEX 两票 |
| 标价喂给 TVL | 池 TVL 在链上标价刷新完成前使用上一轮值（滞后一个 feed 周期，约 2–6 s） |

---

## 23. 附录：关键常量与默认值

| 项 | 值 |
|---|---|
| 期初权益 | 5,000 USDC |
| DEX/CEX | 50/50 |
| 档位 | 250/500/1,000/2,500/5,000/7,500/10,000 |
| 机会 TTL | 30s |
| 扫描周期 | 1,500ms |
| feed 周期 | 2,000–2,500ms |
| 最小投入 | 100 USD |
| 最大投入 | 5,000 USD |
| 最小池 TVL | 25,000（Base 10,000） |
| 最大池状态年龄 | 8,000ms |
| 单池容量占比 | 10% |
| 最少/最多跳 | 2 / 5 |
| 逐档数量 | 7 |
| 标价主源 | 链上最深计价池 spot |
| 标价校验源 | Chainlink（ETH/BTC/LINK/ARB/USDC） |
| 标价分歧阈值 | 50 bps（超阈值机会降级为观察） |
| 测试 | 217 |

---

*本 TDD 为 v2.0 实现基线；未实现/受外部条件限制的项已在正文标注。*
