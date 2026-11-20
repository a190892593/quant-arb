# 原子套利与 CEX–DEX 对冲交易系统
## 总体技术设计说明书（TDD v1.2）

**文档状态：待用户确认**  
**部署基线：本机运行，非容器化**  
**业务基线：统一核心引擎，Research / Paper / Live Adapter 隔离**

---

## 1. 技术目标与架构原则

### 1.1 核心原则

1. 统一策略计算引擎。
2. DEX 原子套利和 CEX–DEX 链外对冲使用不同执行模型。
3. Research、Paper 和 Live 使用统一策略对象、机会对象、风险规则和交易计划格式。
4. Paper 与 Live 使用独立 Adapter、账户、账本和权限域。
5. 所有市场数据保留来源、时间戳和质量标记。
6. 所有历史审计尽量可重放、可复现。
7. 资金、成交、费用和库存采用可审计账本。
8. 先保证数据和执行模型可靠，再追求扫描速度。
9. 第一阶段不接入真实交易签名和实盘 API 下单。
10. 后续迁移云端不得改变策略核心行为。

### 1.2 模块分层

**数据层**

- EVM RPC Connector。
- Block / Transaction Indexer。
- DEX Pool State Engine。
- CEX WebSocket / REST Connector。
- CEX Order Book Engine。
- Historical Dataset Importer。
- Data Quality Monitor。

**策略层**

- DEX Triangle Searcher。
- DEX Multi-Pool Searcher。
- CEX–DEX Spread Searcher。
- Route Optimizer。
- Liquidity / Depth Estimator。
- Funding Cost Estimator。
- Execution Cost Model。
- Opportunity Scorer。

**模拟与执行层**

- REVM / Local EVM Simulation。
- DEX Paper Adapter。
- CEX Paper Order Adapter。
- Cross-Venue Hedge Simulator。
- Trade Plan Manager。
- Future DEX Live Adapter。
- Future CEX Live Adapter。

**风险与账户层**

- Pre-Trade Risk Engine。
- In-Flight Risk Monitor。
- Post-Trade Reconciliation。
- Exposure Monitor。
- Inventory Manager。
- Ledger Engine。
- Kill Switch。

**应用层**

- Axum API Server。
- React + TypeScript Frontend。
- WebSocket Event Gateway。
- Local CLI / PowerShell Launcher。

---

## 2. 第一阶段本地部署架构

### 2.1 不使用容器

第一阶段不部署 Docker、Docker Compose、Kubernetes 或容器化依赖。

建议的本机进程：

1. `arb-api`：Rust API 服务、WebSocket 服务和任务管理。
2. `arb-worker`：行情采集、区块索引、池状态更新和扫描任务。
3. `arb-audit`：历史数据导入、区块重放、订单簿回放和审计报告。
4. `arb-paper`：Paper Trading 与账本结算。
5. `arb-ui`：React / TypeScript 前端开发服务。
6. PostgreSQL：本机数据库服务。

第一阶段允许将 `arb-api`、`arb-worker` 和 `arb-paper` 集成在同一个 Rust 后端进程中，通过 Tokio task 和 bounded channel 传递高频事件，以降低本机部署复杂度。历史审计保留独立 CLI 入口，避免大规模重放任务阻塞实时扫描。

### 2.2 本地技术建议

- 后端：Rust + Tokio + Axum。
- 前端：React + TypeScript。
- 数据库：PostgreSQL。
- 实时进程内消息：Tokio mpsc / broadcast channel。
- 历史原始数据：本地 Parquet 分区文件。
- 离线分析：DuckDB。
- 链上模拟：REVM / 本地 EVM Fork。
- 合约开发与测试：Solidity + Foundry。
- 监控：本地结构化日志、指标页面和健康检查。

第一阶段不把 Redis 和 ClickHouse 作为强制依赖。只有在数据量、并发吞吐或查询性能实测不足时才引入，以免本机启动、维护和恢复复杂度过高。

### 2.3 Windows 本机运行

建议 Windows 11 优先：

- PowerShell 脚本启动和停止服务。
- PostgreSQL 作为本机服务。
- 前后端由本机进程运行。
- 合约测试环境可根据工具链兼容性使用本机 WSL2 Ubuntu。
- 配置和密钥文件不得提交到 Git。
- 重要凭据优先使用 Windows Credential Manager / DPAPI 或受保护的秘密存储方式。
- 本机休眠、断网、系统更新和重启时必须停止 Paper 订单状态推进，并将行情数据标记为断档。

本机模式是第一阶段开发和研究运行方式，不等于适合未来全天候实盘。后续如果要求持续 24/7 运行，应单独评估稳定主机或云服务器。

---

## 3. 链数据架构

### 3.1 Chain Registry

字段：

- chain_id
- chain_name
- native_token
- rpc_primary
- rpc_secondary
- websocket_endpoint
- block_time_estimate
- finality_model
- gas_model
- chain_status
- data_quality
- adapter_version
- enabled_at

链状态：

`DISCOVERED → DATA_VALIDATED → HISTORICAL_READY → PAPER_READY → LIVE_CANDIDATE → LIVE_APPROVED`

任何状态转换必须有对应验证记录。

### 3.2 Block / Transaction Indexer

采集：

- Block Header。
- Block Timestamp。
- Base Fee / Priority Fee。
- Transaction。
- Receipt。
- Logs。
- Transaction Index。
- Gas Used。
- Effective Gas Price。
- Revert Status。
- Contract Addresses。

重放必须遵循：

`Block Number → Transaction Index → Log Index`

禁止使用区块结束状态代替交易执行前状态来计算交易是否可实现。

### 3.3 数据源健康

- 主 RPC 与备用 RPC。
- 最新区块落后程度。
- 不同 RPC 区块哈希一致性。
- WebSocket 断开与重连次数。
- 历史数据区间缺口。
- 归档状态可读取性。
- 请求超时、错误率和延迟分位数。

归档 RPC 不可访问时，系统只能对可用数据区间进行审计，不能编造缺失历史状态。

---

## 4. DEX 协议与路由架构

### 4.1 Adapter 接口

统一抽象：

- `protocol_id`
- `protocol_version`
- `factory`
- `router`
- `pool_address`
- `token0`
- `token1`
- `fee_tier`
- `get_pool_state()`
- `quote_exact_in()`
- `quote_exact_out()`
- `simulate_swap()`
- `build_execution_leg()`

具体协议适配：

- Uniswap V2 类。
- Uniswap V3 类。
- StableSwap 类。
- Weighted Pool 类。
- 其他经验证协议。

V4、Hook 和定制化池不能直接复用普通 V3 逻辑，必须单独验证池地址、回调路径、Hook 行为和状态更新方式。

### 4.2 路由搜索

路由对象：

- 起始 Token。
- 最终 Token。
- 交易腿序列。
- Pool / Protocol。
- 输入数量。
- 每腿预期输出。
- 每腿最低输出。
- 预计 Gas。
- 费用明细。
- 路线状态。
- 模拟可信等级。

仅允许同链原子策略进入链上 Executor。

跨链路线不得标记为原子套利。

---

## 5. CEX 市场数据架构

### 5.1 CEX Adapter

统一数据接口：

- `fetch_instruments()`
- `fetch_trading_rules()`
- `fetch_fee_schedule()`
- `subscribe_orderbook_snapshot()`
- `subscribe_orderbook_delta()`
- `subscribe_trades()`
- `get_server_time()`
- `normalize_symbol()`

后续实盘适配接口预留：

- `place_order()`
- `cancel_order()`
- `query_order()`
- `query_fills()`
- `query_balances()`
- `subscribe_private_orders()`

第一阶段只启用公共行情接口和 Paper Adapter，实盘下单接口不得被调用。

### 5.2 Order Book Engine

维护：

- 交易所。
- 交易对。
- Snapshot ID / Sequence。
- Bid Levels。
- Ask Levels。
- Exchange Timestamp。
- Local Receive Timestamp。
- 数据延迟。
- 数据完整状态。
- 最近一次完整同步时间。

处理规则：

1. 启动订阅后取得完整 Snapshot。
2. 按交易所要求的序列号顺序应用增量。
3. 发现缺口、乱序或断线时，盘口立即标记为 INVALID。
4. 重新拉取 Snapshot 并恢复增量同步。
5. 只有完整且未超时的盘口才能生成可执行机会。
6. 历史盘口重放必须保持原始时间顺序。

### 5.3 CEX 成交模型

成交估算必须消耗盘口深度。

买入按 Ask 价格档位逐级消耗，卖出按 Bid 价格档位逐级消耗。

成本包含：

- 手续费。
- 价格冲击。
- IOC 剩余量撤销。
- FOK 未完全成交。
- 订单提交延迟。
- 行情延迟。
- 数据缺口。
- 补偿平仓损耗。

不能使用 Mid Price 或 K 线价格直接假设整笔订单成交。

---

## 6. 统一策略引擎与机会对象

### 6.1 标准机会对象

- opportunity_id
- strategy_type
- chain_id
- dex_route
- cex_exchange
- cex_symbol
- direction
- target_notional
- required_dex_inventory
- required_cex_inventory
- theoretical_edge
- executable_gross_profit
- total_cost
- conservative_net_profit
- stress_net_profit
- risk_score
- data_quality
- quote_timestamp
- quote_block_number
- status

### 6.2 成本模型

统一净收益：

`净收益 = 实际回款价值 - 实际投入价值 - DEX 费用 - CEX 费用 - Gas - 闪电贷费用 - Builder/优先费 - 滑点 - 对冲补偿成本 - 库存再平衡成本 - 其他执行成本`

所有成本均需明确币种和换算时点。

对 CEX–DEX 策略，额外计入：

- 盘口深度损耗。
- 两边时间差。
- 非同步成交风险。
- 残余敞口的风险准备金。
- 资金分布造成的机会成本。

### 6.3 评分建议

- 保守净利润：25 分。
- 模拟可信度：20 分。
- 流动性容量：15 分。
- 执行竞争：15 分。
- 数据新鲜度：10 分。
- Token / Pool / Exchange 风险：10 分。
- 历史执行表现：5 分。

评分不能替代硬性风险拦截。即使评分很高，触发硬风控时仍然必须拒绝。

---

## 7. CEX–DEX 执行状态机

此状态机先用于 Paper Trading，未来实盘也使用同样的业务状态。

`DETECTED → VALIDATED → PLANNED → HEDGE_SUBMITTED / DEX_SUBMITTED → PARTIALLY_FILLED / LEG_CONFIRMED → SECOND_LEG_PENDING → HEDGED → RECONCILED`

异常状态：

- `REJECTED`
- `EXPIRED`
- `ORDER_STATUS_UNKNOWN`
- `PARTIAL_HEDGE`
- `DEX_REVERTED`
- `HEDGE_FAILED`
- `UNWIND_REQUIRED`
- `UNWIND_FAILED`
- `MANUAL_RECOVERY_REQUIRED`

其中 `MANUAL_RECOVERY_REQUIRED` 是异常处置状态，不代表系统正常流程变为逐笔审批。

### 7.1 先成交 CEX，再执行 DEX

Paper 流程：

1. 重新验证 CEX 盘口和 DEX 区块状态。
2. 模拟带价格保护的 IOC/FOK 订单。
3. 获取实际模拟成交数量。
4. 按成交数量重新计算 DEX 交易计划。
5. 重新执行 DEX 模拟。
6. DEX 模拟通过且净利润符合要求，才模拟链上交易。
7. DEX 失败或回滚时，模拟 CEX 反向补偿订单。
8. 计算补偿后的净盈亏和剩余敞口。

### 7.2 先执行 DEX，再成交 CEX

Paper 流程：

1. 验证 DEX 交易计划和 CEX 盘口。
2. 模拟 DEX 原子交易。
3. 根据链上模拟实际输出量，计算 CEX 对冲订单。
4. 模拟 CEX 成交。
5. CEX 部分成交时记录未对冲数量。
6. 超过敞口阈值时模拟立即补偿。
7. 对冲完成后进行两边账本核对。

以上两种方式必须在历史审计和 Paper Trading 中分别统计成交率、回撤、失败损失和敞口时间。最终执行顺序不能仅凭理论判断。

### 7.3 关键限制

- CEX 与 DEX 不是同一事务。
- 不存在跨两个场所的原子回滚。
- CEX 订单 ACK 不等于成交。
- REST 请求超时不代表订单没有提交。
- 状态未知时必须先查询，不得盲目重发相同逻辑订单。
- 使用唯一 Client Order ID 和幂等执行记录。
- 交易数量以真实成交数量为准，不以请求数量为准。
- CEX 库存和链上钱包库存分开核算。
- 不将链上充值、提现或跨链桥转账纳入套利原子路径。

---

## 8. DEX 执行合约架构

### 8.1 合约组件

- `ArbExecutor`：原子交易入口和路线控制。
- `FundingProviderRegistry`：闪电贷提供方登记。
- `AaveV3ReceiverAdapter`：Aave V3 回调适配。
- `ERC3156ReceiverAdapter`：ERC-3156 回调适配。
- `DexAdapterRegistry`：允许的 DEX 适配器登记。
- `TokenRegistry`：允许 Token 登记。
- `Treasury`：资金归集和受限提款。
- `ExecutionPlanVerifier`：签名、Nonce、期限及路线计划校验。

各闪电贷协议回调接口不同，必须使用独立 Receiver Adapter，不能假定一个通用 Callback ABI 能兼容所有协议。

### 8.2 自有资金模式

1. 读取 Executor 起始余额。
2. 验证 Token、路线、数量、期限和 Nonce。
3. 执行 DEX 交易腿。
4. 检查最终资产回款。
5. 要求回款覆盖原始余额与最低利润。
6. 记录执行结果。
7. 归还或保留利润，按 Treasury 规则处理。

### 8.3 闪电贷模式

1. 用户或授权执行账户调用 Executor。
2. Executor 校验交易计划。
3. 记录借款前的基准余额。
4. 向经过登记的 Provider 发起闪电贷。
5. Provider 转入资金并触发专属回调。
6. Receiver 校验 Provider、Initiator、Token、金额、费用和参数哈希。
7. 执行授权 DEX 路线。
8. 检查回款是否覆盖本金、费用和最低利润。
9. 仅授权 Provider 拉取精确应还金额。
10. Provider 拉款成功，整笔交易成功；否则整笔回滚。

### 8.4 回调安全

- 回调调用方必须是登记的 Provider。
- Initiator 必须是本 Executor。
- 资产和借款数量必须与发起记录一致。
- 回调只能在预期状态下执行一次。
- 同一交易不能重复进入执行流程。
- Callback Data 必须经过计划哈希校验。
- 采用明确状态机控制预期回调，不依赖简单的通用重入锁覆盖所有场景。

### 8.5 执行限制

- 不允许任意外部调用目标。
- 不允许任意 Delegatecall。
- 不允许用户传入未经批准的 Router 或 Pool。
- 不允许无期限授权。
- 不允许未经批准的 Token。
- 不允许开放任意提款地址。
- 每一跳必须有最低输出约束。
- 最终闭环必须回到指定起始资产。
- 不支持的 Token 行为必须拒绝执行。

### 8.6 利润保护

最低净利润用起始资产计价并在链上检查。

对闪电贷交易，在 Provider 拉走本金和费用之前，Receiver 应要求余额至少为：

`执行前基准余额 + 闪电贷本金 + 闪电贷费用 + 最低利润`

这样，在 Provider 精确拉走应还金额后，Executor 仍应保留原有基准余额和最低利润。

链上合约不能可靠地单独判断所有链下成本，因此完整 USD 净利润还必须由链下风险引擎计算和复核。

---

## 9. 统一资金账本

### 9.1 主要账户类型

- DEX Wallet Paper Account。
- CEX Spot Paper Account。
- Flash Loan Clearing Account。
- Gas Expense Account。
- CEX Fee Expense Account。
- DEX Fee Expense Account。
- Hedge Compensation Account。
- Inventory Rebalance Account。
- Realized PnL Account。
- Unrealized PnL Account。

### 9.2 账本原则

使用不可随意覆盖的流水记录。

每笔交易包括：

- 资金来源。
- 资金去向。
- 交易前余额。
- 交易后余额。
- 成交数量。
- 成交价格。
- 费用。
- 时间戳。
- 策略 ID。
- Opportunity ID。
- Trade Plan ID。
- 链交易 Hash 或 CEX 模拟订单 ID。

### 9.3 CEX–DEX 库存管理

库存净值按所有场所合并计算，同时保留场所隔离。

系统分别报告：

- CEX Base Asset。
- CEX Quote Asset。
- DEX Base Asset。
- DEX Quote Asset。
- 合并净持仓。
- 跨场所净敞口。
- 可用余额。
- 冻结余额。
- 待结算余额。

库存再平衡单独生成任务，不允许把再平衡转账费用隐藏在策略利润里。

---

## 10. 数据库设计

在原有表基础上增加以下主要实体：

### 链和 DEX

- `chains`
- `dex_protocols`
- `dex_adapters`
- `tokens`
- `pools`
- `pool_states`
- `swap_events`
- `blocks`
- `transactions`
- `receipts`

### CEX

- `cex_exchanges`
- `cex_instruments`
- `cex_trading_rules`
- `cex_fee_schedules`
- `cex_orderbook_snapshots`
- `cex_orderbook_deltas`
- `cex_trades`
- `cex_data_health`

### 套利与模拟

- `strategies`
- `routes`
- `opportunities`
- `simulations`
- `trade_plans`
- `hedge_plans`
- `hedge_legs`
- `paper_orders`
- `paper_fills`
- `execution_events`
- `unwind_events`

### 资金和风险

- `funding_providers`
- `funding_transactions`
- `accounts`
- `account_balances`
- `strategy_ledgers`
- `ledger_entries`
- `inventory_snapshots`
- `inventory_rebalances`
- `trades`
- `trade_legs`
- `risk_events`
- `system_controls`
- `system_configs`
- `audit_logs`

大体积原始盘口增量和链上原始数据可采用本机 Parquet 分区文件；PostgreSQL 保存索引、元数据、摘要和交易账本。

---

## 11. API 设计

### 系统

- `GET /system/status`
- `GET /system/health`
- `GET /system/config`
- `GET /system/controls`

### 链与 DEX

- `GET /chains`
- `GET /chains/{chain_id}/health`
- `GET /dex/protocols`
- `GET /dex/pools`

### CEX

- `GET /exchanges`
- `GET /exchanges/{exchange_id}/instruments`
- `GET /markets/orderbook`
- `GET /markets/data-health`

### 扫描与机会

- `POST /scanner/start`
- `POST /scanner/stop`
- `GET /scanner/status`
- `GET /opportunities`
- `GET /opportunities/{id}`
- `GET /opportunities/{id}/scenarios`

### Paper

- `GET /paper/accounts`
- `GET /paper/positions`
- `GET /paper/trades`
- `GET /paper/hedges`
- `GET /paper/performance`
- `POST /paper/reset`

### 风险和资金

- `GET /risk/status`
- `GET /risk/events`
- `POST /risk/pause`
- `POST /risk/resume`
- `GET /inventory`
- `GET /ledger`

### 历史审计

- `POST /audit/jobs`
- `GET /audit/jobs`
- `GET /audit/jobs/{id}`
- `GET /audit/reports/{id}`

WebSocket 实时推送：

- 市场行情。
- 机会发现。
- 风控拒绝。
- Paper 订单状态。
- 对冲状态。
- 资金变化。
- 账本事件。
- 数据连接异常。

---

## 12. 历史审计系统

### 12.1 DEX 数据

优先使用归档 RPC 或具备历史状态能力的数据供应商。

必须保留：

- 区块编号。
- 交易顺序。
- 交易执行前状态。
- Pool 状态。
- Gas。
- 实际交易结果。
- 失败状态。

### 12.2 CEX 数据

需要：

- 历史订单簿 Snapshot。
- 增量深度数据。
- 逐笔成交。
- 手续费规则。
- 交易规则。
- 时间同步信息。

仅有 K 线或逐笔成交时，不能准确恢复完整盘口深度和实际排队成交情况，应降低可信度。

若交易所不提供目标历史区间的完整 L2 订单簿，应使用可合法取得的历史订单簿供应商数据，或从系统启动日起持续记录实时盘口。缺少历史盘口的数据区间只能用于初筛，不能作为高可信可成交利润结论。

### 12.3 模型等级

- M0：理论价差。
- M1：历史状态插入模拟。
- M2：保守竞争和深度成交模型。
- M3：压力模型，包括延迟、滑点、部分成交、回滚和补偿。

### 12.4 审计状态

- PASS：证据充分且通过全部预设标准。
- WATCH：存在可行性信号但数据或样本不足。
- REJECT：成本、容量、风险或结果不符合要求。
- PAPER：仅在模拟环境保留观察，不进入真实执行。

---

## 13. 测试体系

### 单元测试

- DEX 报价。
- CEX 深度消耗。
- 费用计算。
- Token 精度。
- 交易计划哈希。
- 风控规则。
- 账本一致性。

### 集成测试

- 多链 RPC。
- DEX Adapter。
- CEX Adapter。
- 历史数据导入。
- Paper 执行。
- 对冲状态机。
- 数据断线和重连。

### 合约测试

- 正常原子闭环。
- 每一跳最低输出失败。
- 借款金额篡改。
- 错误 Provider 回调。
- 错误 Initiator。
- 重复回调。
- 过期计划。
- Nonce 重放。
- 不允许的 Token / Router / Pool。
- 闪电贷偿还不足。
- 最低利润不满足。
- 恶意 Token 行为。
- 重入和资金转移边界。

### 压力测试

- RPC 延迟。
- CEX WebSocket 断线。
- 盘口序列缺失。
- 数据乱序。
- 交易所响应超时。
- 订单状态未知。
- CEX 部分成交。
- DEX 交易回滚。
- Paper 服务重启恢复。
- PostgreSQL 断连。
- 本机时钟漂移。

---

## 14. 安全与密钥隔离

第一阶段：

- 不存储实盘私钥。
- 不要求 CEX 下单 API Key。
- 外部 RPC 凭据按秘密信息管理。
- 配置文件不得提交版本控制。
- Paper Adapter 没有调用 Live Adapter 的能力。

未来实盘：

- DEX 签名器独立。
- CEX API 凭据独立。
- CEX API Key 不开通提现权限。
- CEX 下单权限与资产转出权限分离。
- 对账户、链、策略和额度配置建立独立白名单。
- 任何 Live 请求必须经风险引擎通过。

---

## 15. 可观测性与恢复

核心指标：

- RPC 延迟与错误率。
- CEX WebSocket 延迟。
- 盘口完整率。
- 数据年龄。
- 扫描延迟。
- 模拟耗时。
- 机会到期率。
- Paper 成交率。
- 对冲完成率。
- 部分成交率。
- DEX 回滚率。
- 平均补偿成本。
- 最大未对冲敞口。
- 账本差异。
- 数据断档长度。

本机重启后：

1. 恢复 PostgreSQL。
2. 校验账本和配置。
3. 重新同步链头。
4. 重新同步 CEX 订单簿 Snapshot。
5. 检查上次停止前的未完成 Paper 交易。
6. 重新初始化扫描任务。
7. 在数据完整前禁止产生有效的可执行机会。

---

## 16. 关键技术边界

- 不能把跨链套利宣称为同一交易原子执行。
- 不能把 CEX 与 DEX 的两腿交易宣称为原子执行。
- 不能用中间价代替订单簿深度。
- 不能用区块结束状态伪装交易前状态。
- 不能把闪电贷本金计为收益。
- 不能把失败回滚理解为没有 Gas 损失。
- 不能把 API 请求成功等同于订单成交。
- 不能因历史数据不完整而补造收益数据。
- 不能把模拟盈利直接推断为未来实盘盈利。

---

## 17. 第一阶段实施顺序（确认后）

1. 本地开发环境与数据库。
2. Chain Registry 和 DEX 数据采集。
3. CEX Market Data Adapter。
4. 统一数据模型和历史数据存储。
5. 历史审计引擎。
6. DEX 策略扫描器。
7. CEX–DEX 策略扫描器。
8. DEX 模拟执行和 CEX 订单簿模拟执行。
9. 对冲状态机和库存管理。
10. 统一 Paper Trading 与账本。
11. 风险控制台和前端。
12. 合约本地测试、Fork 测试和安全评审。
13. 全流程验收报告。

**在用户确认 PRD、TDD、合约设计和风险参数前，不开始上述编码任务。**

---

## 18. 技术参考

- Aave V3 Flash Loans：<https://aave.com/docs/aave-v3/guides/flash-loans>
- ERC-3156 Flash Loans：<https://eips.ethereum.org/EIPS/eip-3156>
- Binance Spot API：<https://developers.binance.com/docs/binance-spot-api-docs>
- OKX API V5：<https://www.okx.com/docs-v5/en/>
- Uniswap V3：<https://docs.uniswap.org/contracts/v3/overview>