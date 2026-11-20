# DEX 多池多跳套利扫描引擎
## 独立产品需求与详细开发规则（PRD + 技术设计基线）

**文档版本：** v1.0  
**产品名称：** DEX Multi-Pool Multi-Hop Arbitrage Scanner  
**产品类型：** EVM 链上原子套利机会搜索引擎  
**当前阶段：** 需求设计与技术方案确认  
**部署方式：** Local First，本地运行所有服务  
**交易场所范围：** 仅 DEX，不接入 CEX  
**执行范围：** 同链、单笔 EVM 交易内完成闭环  
**首期目标：** 历史数据审计、实时机会扫描、精确模拟、Paper Trading

---

# 第一章：产品定位与设计目标

## 1.1 产品定位

本系统是一套专门针对 EVM 链的 DEX 多池多跳套利扫描引擎。

它通过建立链上 Token—Pool 交易图，持续发现不同 DEX 协议、不同流动性池之间的兑换价格差异，自动搜索可以回到起始资产的套利闭环。

系统不局限于传统的三角套利，也不局限于两个池子之间的跨池套利。

搜索范围包括：

1. 同一交易对的双池套利。
2. 同一交易对的多池套利。
3. 三角套利。
4. 四跳、五跳及更多跳数的闭环套利。
5. 跨 DEX 协议套利。
6. 同一 DEX 不同费率池套利。
7. 不同 AMM 类型之间的混合套利。
8. 稳定币多池套利。
9. 包装资产及经过审核的同链映射资产套利。
10. 闪电贷资金支持的多池多跳套利机会。

所有机会必须满足：

- 同一条 EVM 链。
- 起始资产与最终资产相同。
- 所有兑换步骤在同一笔链上交易中执行。
- 不依赖跨链桥。
- 不依赖 CEX。
- 扣除全部交易成本后仍具有正向预期净收益。
- 满足资金、流动性、Token 安全和执行风险限制。

## 1.2 核心目标

系统需要解决五个核心问题：

**目标一：尽可能完整地发现套利路径。**

不能只搜索 ETH/USDC 两个池子，也不能把搜索范围固定为三角套利。系统需要从已发现和已验证的池子中构建交易图，自动搜索多跳闭环。

**目标二：计算真实可成交利润。**

不能用 Token 的现货报价或池子的简单价格比判断盈利。必须根据实际投入数量，计算每一跳的真实 AMM 输出。

**目标三：寻找最优投入金额。**

同一条路线在 500 USDC、1,000 USDC、5,000 USDC 和 10,000 USDC 下可能具有完全不同的利润表现。

系统必须计算不同资金规模下的回款、净利润和资金利用率。

**目标四：降低无效机会数量。**

通过路线去重、池子质量筛选、报价新鲜度、流动性过滤、精确模拟和风险评分，尽量避免把不可执行的理论价差展示为真实机会。

**目标五：为后续原子套利执行提供标准化数据。**

扫描引擎只负责发现、计算、排序和验证机会。未来执行模块可以使用统一的 Route、TradePlan 和 SimulationResult 数据结构。

## 1.3 非目标

以下内容明确不属于本引擎：

- CEX 行情接入。
- CEX 现货交易。
- CEX 永续合约。
- CEX–DEX 对冲。
- 跨交易所套利。
- 跨链套利。
- 资金费率套利。
- 期现基差套利。
- 做市策略。
- 自动实盘交易执行。
- 主网私钥签名管理。

未来可以由其他独立模块实现，但不得进入本引擎的核心搜索逻辑。

---

# 第二章：套利类型与路线定义

## 2.1 统一路线模型

所有套利策略统一抽象成一个闭环：

起始资产 A → 资产 B → 资产 C → …… → 资产 A

设起始投入金额为：

A₀

每个 Pool 根据其 AMM 类型、当前状态、交易手续费和输入数量计算下一跳输出：

A₁ = Swap₁(A₀)

A₂ = Swap₂(A₁)

……

Aₙ = Swapₙ(Aₙ₋₁)

最终回款：

A_final = Swapₙ(...Swap₂(Swap₁(A₀)))

毛利润：

GrossProfit = A_final - A₀

净利润：

NetProfit = A_final - A₀ - GasCost - FundingFee - OtherCosts

如果使用自有资金，不应扣除虚构的闪电贷费用。

如果使用闪电贷，必须计入对应借贷协议的实际费用。

## 2.2 路线类型

### 类型 A：双池跨池套利

典型路径：

USDC → ETH（Pool A）

ETH → USDC（Pool B）

特点：

- 两个 Pool。
- 一个中间 Token。
- 起始资产与终止资产相同。
- 主要利用相同交易对的池间价格差。

### 类型 B：多池跨池套利

典型路径：

USDC → ETH（Pool A）

ETH → USDC（Pool B）

也可以扩展为：

USDC → ETH（Pool A）

ETH → WBTC（Pool B）

WBTC → USDC（Pool C）

此类路线可能同时包含跨池和三角套利特征。

### 类型 C：三角套利

典型路径：

USDC → ETH → WBTC → USDC

重点识别三个资产之间的兑换汇率不一致。

### 类型 D：多跳闭环套利

典型路径：

USDC → ETH → WBTC → DAI → USDT → USDC

支持配置最大跳数，不应把五跳视为固定上限。

但跳数增加会提高：

- Gas 消耗。
- 计算复杂度。
- 执行失败概率。
- 状态变化影响。
- 路线竞争程度。

因此必须设置最大跳数和搜索预算。

### 类型 E：跨协议套利

例如：

Uniswap V2 → Uniswap V3 → Curve → Balancer

允许同一条路线使用不同 DEX 协议的 Pool。

### 类型 F：混合 AMM 套利

允许一条路线包含：

- V2 Constant Product。
- V3 Concentrated Liquidity。
- StableSwap。
- Weighted Pool。
- 其他经过适配和验证的 AMM 类型。

V4 Hook、特殊动态费用和自定义 AMM 必须经过专门适配和安全验证后才能开放。

---

# 第三章：链与协议支持规则

## 3.1 链支持

系统采用 All EVM Chains 的可扩展架构。

但 All EVM Chains 表示架构支持扩展，并不代表所有链启动时都具备实盘准入资格。

每条链必须具有独立的 Chain Registry。

核心字段：

- Chain ID。
- 网络名称。
- 原生 Gas Token。
- RPC 节点列表。
- WebSocket 节点。
- 区块高度。
- 最终性配置。
- Gas 价格来源。
- 区块时间统计。
- 归档数据能力。
- 链健康状态。
- 是否允许历史审计。
- 是否允许实时扫描。
- 是否允许 Paper Trading。
- 是否允许未来实盘执行。

链的状态：

DISCOVERED：已登记。

RESEARCH：研究阶段。

AUDIT：历史数据审计中。

PAPER：允许模拟。

LIVE_READY：通过实盘准入审核。

DISABLED：禁用。

首期可以从以太坊及其他经过数据和协议适配验证的 EVM 网络开始，但不能把未经验证的网络直接纳入有效套利统计。

## 3.2 DEX 协议支持

协议适配采用插件式架构。

第一批优先支持：

| AMM 类型 | 支持要求 | 优先级 |
|---|---|---|
| Uniswap V2 类 | 支持 | P0 |
| Uniswap V3 类 | 支持 | P0 |
| Curve StableSwap | 支持 | P1 |
| Balancer Weighted | 支持 | P1 |
| Uniswap V4 | 独立 Hook 适配 | P2 |
| 其他自定义 AMM | 插件扩展 | P2 |

每个协议适配器必须提供：

1. Pool 发现。
2. Pool 地址验证。
3. Token 地址识别。
4. Token 精度获取。
5. Pool 状态获取。
6. Swap 报价。
7. 手续费计算。
8. 精确输出计算。
9. 状态更新。
10. 历史数据解析。
11. 交易模拟兼容性。

禁止使用一个通用报价公式处理所有 AMM。

不同 AMM 必须实现各自的精确报价模型。

## 3.3 Token 白名单

Token 必须建立独立注册表。

至少包含：

- Chain ID。
- Token 合约地址。
- Symbol。
- Name。
- Decimals。
- 合约代码哈希。
- 是否代理合约。
- 是否存在暂停功能。
- 是否存在黑名单功能。
- 是否存在转账税。
- 是否存在重基准机制。
- 是否存在转账限制。
- 是否经过审核。
- 风险等级。

Token 状态：

UNKNOWN、OBSERVING、APPROVED、RESTRICTED、REJECTED。

首期默认只允许 APPROVED Token 进入高可信机会列表。

禁止仅凭 Symbol 相同认定两个 Token 是同一种资产。

默认排除：

- 未知税费 Token。
- 转账金额不确定的 Token。
- 黑名单或冻结风险明显的 Token。
- 无法正确模拟的 Token。
- 代码行为不透明且未审核的 Token。

---

# 第四章：交易图与路线搜索引擎

## 4.1 交易图模型

系统采用有向多重图（Directed Multigraph）。

图中的节点：

Token。

图中的边：

某个 Pool 提供的 Token A → Token B 兑换能力。

由于一个交易对可能有多个 Pool，因此两个 Token 之间可以存在多条不同的边。

每条边必须绑定唯一 Pool。

例如：

ETH → USDC：

- Uniswap V2 ETH/USDC。
- Uniswap V3 ETH/USDC 0.05%。
- Uniswap V3 ETH/USDC 0.30%。
- Curve ETH/USDC。
- 其他经过验证的 Pool。

不同 Pool 不能合并成一条没有来源的抽象报价边。

## 4.2 图构建规则

系统启动后：

1. 从已注册 DEX Factory 获取 Pool。
2. 从历史事件发现新 Pool。
3. 读取 Pool Token。
4. 校验 Token 合约。
5. 获取 Pool 状态。
6. 创建双向兑换边。
7. 为边关联 AMM 适配器。
8. 将状态写入本地缓存。
9. 监控后续 Swap、Mint、Burn、Sync 等状态变化。

注意：双向边只是搜索结构，不代表两个方向的输出价格相同。

每个方向都必须独立计算手续费、滑点和输出。

## 4.3 闭环搜索规则

搜索从指定起始资产开始。

例如：

USDC → ETH → WBTC → USDC

必须满足：

- 最终 Token 地址与起始 Token 地址相同。
- 每一跳均有经过验证的 Pool。
- 所有 Pool 属于同一 Chain ID。
- 不允许在一条路线中重复使用同一个 Pool，除非未来专门设计并验证重复 Pool 状态模拟。
- 默认不允许重复中间 Token。
- 不允许使用未知 Token。
- 不允许跨链。
- 不允许路线依赖外部链下交易。

默认采用简单闭环搜索，避免重复节点产生大量无意义循环。

## 4.4 跳数配置

系统必须允许设置：

- 最小跳数。
- 最大跳数。
- 最大 Pool 数。
- 最大候选路线数量。
- 单次搜索最大计算时间。
- 单次搜索最大 CPU 预算。
- 单次搜索最大内存预算。

初始研究参数建议：

| 参数 | 初始值 |
|---|---:|
| 最小跳数 | 2 |
| 最大跳数 | 5 |
| 最大 Pool 数 | 5 |
| 单次候选路线预算 | 100,000 |
| 单次搜索超时 | 可配置 |
| 默认起始资产 | USDC、USDT、WETH、DAI 等审核资产 |

以上是工程初始值，不是经过实测得出的最优参数。

必须支持后续扩大至 6、7、8 跳，但要通过性能测试和历史审计决定是否开放。

## 4.5 搜索算法

建议采用分层搜索：

第一层：Token 图快速筛选。

第二层：深度优先搜索（DFS）或有界环路枚举。

第三层：基于流动性、协议和风险的候选路线过滤。

第四层：精确报价计算。

第五层：投入金额优化。

第六层：EVM 状态模拟。

第七层：机会排序。

对于候选路线数量过多的情况，采用：

- 搜索深度限制。
- 节点访问限制。
- 边数量限制。
- 流动性预筛选。
- 路线缓存。
- 价格区间剪枝。
- 计算预算控制。

不能仅依靠 Token 数量和 Pool 数量暴力枚举所有组合。

## 4.6 路线去重

以下路线需要识别重复：

USDC → ETH（A）→ USDC（B）

USDC → ETH（B）→ USDC（A）

它们可能代表两个不同的执行方向，不能直接当成完全相同的机会。

需要建立：

- 无方向路线指纹。
- 有方向路线指纹。
- Pool 序列指纹。
- Token 序列指纹。
- 起始资产指纹。

同一条有方向、同一组 Pool 的路线只能在机会列表中保留一个当前版本。

---

# 第五章：报价和精确利润计算

## 5.1 禁止使用简单现货价格计算利润

错误做法：

DEX A ETH 价格 = 3,000。

DEX B ETH 价格 = 3,020。

直接判断每 ETH 有 20 USDC 利润。

正确做法：

必须根据实际投入金额，依次计算每一跳真实输出。

## 5.2 AMM 报价模型

每种 AMM 必须使用对应的数学模型。

### V2 类 Pool

需要读取：

- Reserve0。
- Reserve1。
- Fee。
- Token Decimals。

使用对应协议的实际费率和整数精度规则计算输出。

### V3 类 Pool

必须考虑：

- sqrtPriceX96。
- Current Tick。
- Liquidity。
- Tick Bitmap。
- Initialized Ticks。
- Fee Tier。
- 跨 Tick 价格变化。
- Token 精度。
- Swap 方向。

不能仅使用当前 Tick 的局部价格估算大额交易。

### StableSwap

必须实现对应 StableSwap 曲线计算：

- Pool balances。
- Amplification 参数。
- Fee。
- 精确整数迭代。
- 资产数量差异。

### Weighted Pool

必须考虑：

- 各 Token 权重。
- Pool balances。
- Swap Fee。
- 最大交易比例。
- 精确数学计算。

### 特殊 AMM

如果协议具有动态手续费、Hook、特殊回调或额外资产转移规则，必须独立适配。

没有可靠报价模型的 Pool 不允许进入高可信机会。

## 5.3 多跳计算

每一跳的输出必须作为下一跳的输入。

例如：

A0 → A1 → A2 → A3

必须执行：

A1 = Quote1(A0)

A2 = Quote2(A1)

A3 = Quote3(A2)

不能将每一跳的独立最佳价格简单相乘后作为真实收益。

## 5.4 投入金额优化

系统必须支持多资金档位计算。

初始资金测试：

- 250 USDC。
- 500 USDC。
- 1,000 USDC。
- 2,500 USDC。
- 5,000 USDC。
- 7,500 USDC。
- 10,000 USDC。

每条路线需要计算：

- 最小盈利投入。
- 最优投入。
- 最大可执行投入。
- 最大净利润。
- 净利润率。
- 资金利用率。
- Gas 占比。

由于 AMM 输出通常不是线性的，最优投入不能通过固定价格差直接计算。

建议采用：

1. 粗粒度区间扫描。
2. 识别可盈利区间。
3. 对可盈利区间进行一维优化。
4. 在整数精度下重新计算。
5. 使用精确模拟验证最终候选金额。

可以使用有界网格搜索、黄金分割搜索或其他经过测试的数值方法。由于不同路线可能存在非光滑区间、Tick 跳变和费率变化，不能无条件假定目标函数全局单峰。

## 5.5 成本计算

净利润必须包含：

NetProfit = FinalOutput - InitialInput - GasCost - FlashLoanFee - BuilderPayment - OtherCosts

其中：

- FinalOutput：最终回款。
- InitialInput：起始本金。
- GasCost：Gas 成本。
- FlashLoanFee：闪电贷费用。
- BuilderPayment：可选排序或 Builder 支付。
- OtherCosts：明确发生且可计量的其他成本。

自有资金模式不计算闪电贷费用。

闪电贷模式必须使用具体 Provider 的费率与回调规则。

失败回滚的交易不能计入成功套利，但必须在 Paper Trading 和风险账本中记录 Gas 损失。

## 5.6 Gas 估算

Gas 不能简单按照固定次数乘以固定费用。

必须通过模拟或可靠的执行模型估算：

- 每个 Router 调用。
- 每次 Swap。
- 每次 Token Transfer。
- 闪电贷借款与还款。
- Executor 合约逻辑。
- 可能的 Approve 或 Permit。
- 回调开销。
- 其他协议特定开销。

同时记录：

- Gas Used。
- Effective Gas Price。
- Gas Token。
- 估算 Gas 成本。
- 估算误差。
- 估算时间。
- 链上区块高度。

---

# 第六章：精确状态模拟

## 6.1 模拟分级

系统采用四级模拟：

**M0：理论价格模拟**

基于当前 Pool 状态和报价公式计算。

用途：快速过滤。

不允许将 M0 结果标记为已验证套利机会。

**M1：状态一致性模拟**

使用统一区块状态和 Pool 数据，逐跳计算实际输入输出。

用途：验证多跳计算的一致性。

**M2：本地 EVM 精确模拟**

使用本地 EVM 模拟器执行完整候选交易。

要求：

- 相同 Chain ID。
- 相同区块状态。
- 相同 Pool 地址。
- 相同交易顺序。
- 相同 Router/Adapter 逻辑。
- 相同 Token 余额和授权条件。
- 相同 Gas 与执行参数。

建议使用 REVM 等本地 EVM 执行环境。

**M3：压力模拟**

测试：

- Gas 上涨。
- 池子状态变化。
- 价格滑点扩大。
- Tick 穿越。
- 部分 Pool 状态变化。
- 交易延迟。
- 闪电贷费用变化。
- 执行竞争。
- 路线中某个 Pool 被提前交易。

## 6.2 状态一致性

一条路线的所有 Pool 报价必须尽量基于同一状态快照。

必须记录：

- Block Number。
- Block Hash。
- Timestamp。
- Pool State Version。
- RPC 来源。
- 状态获取时间。

禁止将不同区块的池子状态拼接为一个所谓的同时刻报价。

如果不同 RPC 返回的数据不一致，必须降低可信度或直接拒绝候选路线。

## 6.3 模拟结果与报价差异

记录：

- Quote Output。
- Simulated Output。
- Difference。
- Difference BPS。
- Gas Estimate Difference。
- Simulation Error。

如果报价模型与 EVM 模拟差异超过配置阈值，路线应标记为 MODEL_MISMATCH。

该路线不能进入高可信机会列表。

---

# 第七章：机会筛选与排序

## 7.1 机会分类

每条机会具有以下状态：

DISCOVERED：发现。

QUOTED：完成报价。

PROFITABLE_ESTIMATE：理论净利润为正。

SIMULATED：已完成 EVM 模拟。

VERIFIED：通过规定的模拟和风险检查。

REJECTED：不符合要求。

EXPIRED：机会过期。

FAILED：模拟或数据处理失败。

## 7.2 最低利润门槛

首期研究基准：

起始资金：5,000 USDC 等值。

最低绝对净利润：可配置。

最低净利润率：可配置。

最低净利润 BPS：可配置。

不能只用利润百分比判断。

例如：

5,000 USDC 的 0.1% = 5 USDC。

但如果 Gas 已经需要 8 USDC，即使毛利润率达到 0.1%，仍然是亏损机会。

## 7.3 机会评分

建议 100 分制：

| 指标 | 权重 |
|---|---:|
| 预计净利润 | 25 |
| 精确模拟可信度 | 20 |
| 流动性容量 | 15 |
| 状态新鲜度 | 10 |
| 执行复杂度 | 10 |
| Pool 与 Token 安全 | 10 |
| 历史模拟表现 | 10 |
| 合计 | 100 |

评分仅用于排序，不能替代硬风控。

任何硬性风险规则触发，都必须直接拒绝或降级机会。

## 7.4 机会记录内容

每条机会必须展示：

- Opportunity ID。
- Chain。
- Start Token。
- Route Type。
- Token Path。
- Pool Path。
- DEX Protocol。
- Swap Count。
- Input Amount。
- Final Output。
- Gross Profit。
- Gas Cost。
- Flash Loan Fee。
- Net Profit。
- Net Profit BPS。
- Simulation Status。
- Risk Score。
- Block Number。
- Quote Timestamp。
- Expiration。
- Rejection Reason。

---

# 第八章：Pool 与 Token 安全控制

## 8.1 Pool 准入

必须检查：

- Factory 是否可信。
- Pool 地址是否与 Factory 创建记录匹配。
- Token 地址是否正确。
- Pool 是否存在可用流动性。
- Pool 是否可正常 Swap。
- Pool 是否存在特殊限制。
- Pool 代码是否发生变化。
- 是否存在异常手续费机制。

## 8.2 Token 风险

必须检测：

- 黑名单机制。
- 暂停交易。
- 转账税。
- 交易限制。
- Rebase。
- 非标准 ERC20 行为。
- 代理升级风险。
- 合约代码变更。
- 无法正常 Transfer 的情况。

无法确认行为的 Token 不得进入高可信路线。

## 8.3 路线级风险

路线安全性取决于所有 Pool 和 Token。

一条路线中任意一个关键 Pool 不可信，就必须降低整条路线的风险等级。

不能因为其中两三个 Pool 很安全，就忽略另一个 Pool 的风险。

---

# 第九章：实时扫描引擎

## 9.1 实时数据来源

数据模块需要支持：

- RPC。
- WebSocket。
- 区块事件。
- Pool 状态读取。
- 新 Pool 发现。
- Swap 事件订阅。
- Mint/Burn/Sync 等事件。
- 区块同步监控。

## 9.2 实时更新流程

1. 监听新区块。
2. 接收相关 Pool 事件。
3. 更新受影响 Pool 状态。
4. 找出与这些 Pool 相关的 Token。
5. 查询受影响路线索引。
6. 对受影响路线重新报价。
7. 进行净利润初筛。
8. 对候选机会进行 EVM 模拟。
9. 更新机会中心。
10. 记录机会生命周期。

不应在每个新区块都无差别地重新搜索所有路线。

必须通过 Pool → Route 的反向索引，优先重新计算真正受影响的路线。

## 9.3 数据新鲜度

每条机会必须记录状态新鲜度。

初始建议：

- 最新区块状态：优先。
- RPC 落后超过配置阈值：降低可信度。
- Pool 状态无法确认：不允许进入 VERIFIED。
- WebSocket 断线：触发重新同步。
- 重连后：使用区块号和事件日志补齐缺失状态。

## 9.4 扫描运行模式

支持：

- 全量初始化扫描。
- 增量实时扫描。
- 指定链扫描。
- 指定 DEX 扫描。
- 指定 Token 扫描。
- 指定 Pool 扫描。
- 指定跳数扫描。
- 指定资金规模扫描。
- 指定路线重算。

扫描任务必须可以暂停、恢复和停止。

---

# 第十章：历史数据审计

## 10.1 审计目标

历史审计不能只统计历史价格差。

必须尽可能重建当时交易发生前的 Pool 状态，判断该路线在当时是否真正能够执行。

## 10.2 审计模式

- 历史 Pool 状态重建。
- 历史路线发现。
- 历史报价。
- 历史资金规模模拟。
- 历史 Gas 成本。
- 历史交易竞争模拟。
- 历史净利润统计。

## 10.3 历史审计窗口

建议：

- 最近 30 天。
- 最近 90 天。
- 最近 180 天。
- 最近 365 天。

不同链的数据完整性必须独立统计。

## 10.4 历史回放规则

应按区块和交易顺序恢复状态。

不能直接使用区块结束时的池子价格，假装这是区块内任意交易之前的可用价格。

应区分：

- 区块开始状态。
- 区块内交易顺序。
- 交易前 Pool 状态。
- 交易后 Pool 状态。
- 区块最终状态。

如果缺少完整事件或历史状态，必须标记数据缺失，不能用插值结果冒充真实回放。

## 10.5 历史统计指标

至少包括：

- 发现机会数量。
- 理论盈利数量。
- 模拟盈利数量。
- 通过率。
- 平均净利润。
- 中位数净利润。
- 最大净利润。
- 最大回撤。
- 每日机会数量。
- 每日净利润。
- 不同资金档位表现。
- 不同跳数表现。
- 不同 AMM 类型表现。
- 不同链表现。
- Gas 占比。
- 失败模拟比例。
- 报价与模拟误差。

## 10.6 审计结论

每个路线类别必须输出：

PASS：通过规定的历史验证标准。

WATCH：有潜力，但数据或表现不足。

REJECT：成本或风险不可接受。

INSUFFICIENT_DATA：数据不足。

审计结论不能仅由理论价差决定。

---

# 第十一章：Paper Trading

## 11.1 模拟原则

Paper Trading 必须使用与未来实盘一致的路线发现、报价、模拟、风控和结算逻辑。

禁止单独开发一套只按理论价格成交的模拟程序。

## 11.2 资金模式

支持：

- 自有资金。
- 闪电贷资金。
- 自有资金 + 闪电贷混合模式。

第一阶段支持三种模式的资金需求和费用模拟。

但闪电贷必须模拟具体 Provider 的借款费用、回调约束和还款要求。

## 11.3 模拟交易过程

1. 扫描发现机会。
2. 完成路线报价。
3. 检查资金。
4. 检查风控。
5. EVM 模拟。
6. 创建 Paper Trade。
7. 记录预期回款。
8. 根据配置模拟执行延迟和状态变化。
9. 计算最终成交结果。
10. 更新 Paper 账户。
11. 生成交易报告。

## 11.4 模拟场景

必须测试：

- 正常成交。
- Gas 上涨。
- Pool 状态变化。
- 报价过期。
- 模拟失败。
- 闪电贷费用变化。
- 路线中途失败。
- Token Transfer 异常。
- 流动性突然下降。
- 交易被其他套利者抢先执行。

Paper Trading 的收益不能直接作为未来实盘收益承诺。

---

# 第十二章：风控规则

首期至少包含以下硬性风控：

1. 最低净利润。
2. 最低净利润率。
3. 最大投入金额。
4. 最大单池资金占比。
5. 最大路线跳数。
6. 最大 Gas 成本。
7. 最大滑点。
8. 最大报价年龄。
9. 最大区块延迟。
10. Token 白名单。
11. Pool 白名单。
12. RPC 健康状态。
13. 模拟结果一致性。
14. 闪电贷还款能力。
15. 日累计亏损。
16. 连续失败次数。
17. 账户资金安全。
18. 紧急暂停。

风控参数必须支持：

- 全局参数。
- 链级参数。
- 协议级参数。
- Token 级参数。
- 路线级参数。
- 资金规模参数。

风控参数修改必须记录操作日志、旧值、新值、修改时间和修改原因。

---

# 第十三章：核心技术架构

建议使用模块化架构。

## 13.1 技术选型

| 模块 | 技术 |
|---|---|
| 前端 | React + TypeScript |
| 后端 | Rust + Axum |
| 异步任务 | Tokio |
| 数据库 | PostgreSQL |
| 高频缓存 | Redis |
| 历史分析 | PostgreSQL 起步，必要时引入 ClickHouse |
| EVM 模拟 | REVM |
| 智能合约测试 | Solidity + Foundry |
| 图搜索 | Rust 自定义图结构 |
| API | REST + WebSocket |
| 监控 | Prometheus + Grafana |

第一阶段不要求 Docker、Kubernetes 或其他容器化部署。

所有服务直接在本地电脑运行。

## 13.2 模块划分

1. Chain Registry。
2. RPC Manager。
3. Block Listener。
4. DEX Protocol Registry。
5. Pool Discovery。
6. Token Registry。
7. Pool State Manager。
8. Trading Graph Builder。
9. Route Search Engine。
10. Route Deduplication。
11. Quote Engine。
12. Input Optimizer。
13. Gas Estimator。
14. EVM Simulation Engine。
15. Opportunity Manager。
16. Risk Engine。
17. Historical Audit Engine。
18. Paper Trading Engine。
19. Ledger。
20. API Gateway。
21. WebSocket Event Service。
22. Monitoring。

## 13.3 核心数据表

至少需要：

- chains
- rpc_endpoints
- dex_protocols
- dex_factories
- tokens
- pools
- pool_states
- swap_events
- block_snapshots
- graph_nodes
- graph_edges
- routes
- route_legs
- route_fingerprints
- scan_jobs
- scan_results
- opportunities
- opportunity_quotes
- simulations
- simulation_legs
- gas_estimates
- risk_rules
- risk_events
- audit_jobs
- audit_results
- paper_accounts
- paper_trades
- paper_trade_legs
- ledger_entries
- system_configs
- audit_logs

数据库必须保存足够的信息，以便复现某条路线在某个历史区块的报价和模拟结果。

---

# 第十四章：前端功能需求

前端需要独立展示 DEX 多池多跳扫描引擎。

## 14.1 总览

显示：

- 扫描状态。
- 活跃链数量。
- 活跃 DEX 数量。
- Pool 数量。
- Token 数量。
- 当前搜索路线数量。
- 理论机会数量。
- 已模拟机会数量。
- VERIFIED 机会数量。
- Paper Trading 收益。
- 系统健康情况。

## 14.2 扫描控制台

支持：

- 总扫描开关。
- 指定链开关。
- 指定 DEX 开关。
- 指定 Token 开关。
- 最大跳数。
- 最小跳数。
- 候选路线预算。
- 资金测试档位。
- 扫描频率。
- 最低净利润。
- 最低流动性。
- 报价有效期。

## 14.3 路线搜索中心

展示：

- 路线类型。
- Token 闭环。
- Pool 序列。
- DEX 协议序列。
- 跳数。
- 资金规模。
- 预计回款。
- 净利润。
- Gas。
- 风险评分。
- 模拟状态。

支持路线筛选、排序和详情查看。

## 14.4 机会详情

必须显示完整路线：

Token A → Pool 1 → Token B → Pool 2 → Token C → Pool 3 → Token A

每一跳展示：

- Pool 地址。
- DEX 名称。
- AMM 类型。
- 输入数量。
- 输出数量。
- 费率。
- 价格冲击。
- 当前流动性。
- 状态区块。

同时展示：

- 毛利润。
- Gas 成本。
- 闪电贷费用。
- 净利润。
- 不同资金规模的收益曲线。
- M0–M3 模拟结果。
- 风险事件。
- 拒绝原因。

## 14.5 历史审计中心

支持：

- 创建审计任务。
- 选择链。
- 选择协议。
- 选择历史窗口。
- 选择最大跳数。
- 选择资金档位。
- 查看审计进度。
- 查看数据完整性。
- 查看历史收益。
- 导出审计报告。

## 14.6 Paper Trading

支持：

- Paper 资金账户。
- 自有资金模拟。
- 闪电贷模拟。
- 资金组合。
- 模拟交易记录。
- 成功和失败统计。
- 账户余额变化。
- 收益曲线。
- 风险事件。
- 交易复盘。

## 14.7 Pool 与 Token 管理

支持：

- Pool 列表。
- Token 列表。
- 协议列表。
- 风险等级。
- 白名单。
- 禁止列表。
- 流动性。
- 数据更新时间。
- 合约代码哈希。

## 14.8 系统监控

显示：

- RPC 状态。
- 区块同步延迟。
- WebSocket 状态。
- 扫描耗时。
- 路线搜索速度。
- CPU 使用率。
- 内存使用率。
- 数据库状态。
- 模拟任务队列。
- 最近错误。

---

# 第十五章：API 与事件规范

建议接口：

GET /api/v1/system/status

GET /api/v1/chains

GET /api/v1/dex/protocols

GET /api/v1/tokens

GET /api/v1/pools

GET /api/v1/scanner/config

POST /api/v1/scanner/start

POST /api/v1/scanner/stop

POST /api/v1/scanner/restart

GET /api/v1/routes

GET /api/v1/routes/{id}

GET /api/v1/opportunities

GET /api/v1/opportunities/{id}

POST /api/v1/simulations

GET /api/v1/simulations/{id}

POST /api/v1/audits

GET /api/v1/audits/{id}

GET /api/v1/paper/accounts

GET /api/v1/paper/trades

GET /api/v1/risk/rules

PUT /api/v1/risk/rules

WebSocket 事件：

- block.updated
- pool.updated
- graph.updated
- scanner.started
- scanner.stopped
- route.discovered
- opportunity.created
- opportunity.updated
- opportunity.expired
- simulation.completed
- simulation.failed
- risk.triggered
- paper.trade.completed

接口必须带有 chain_id、block_number、timestamp 和数据版本等必要字段。

---

# 第十六章：开发阶段与交付顺序

## Phase 0：需求冻结

- 确认产品边界。
- 确认 AMM 类型。
- 确认链准入规则。
- 确认搜索算法。
- 确认报价模型。
- 确认资金规模。
- 确认风控参数。

交付：正式 PRD、TDD、接口规范。

## Phase 1：前端功能界面

- 总览。
- 链管理。
- DEX 管理。
- Token 管理。
- Pool 管理。
- 扫描控制台。
- 路线搜索中心。
- 机会详情。
- 历史审计。
- Paper Trading。
- 风险控制。
- 系统监控。

使用演示数据，先不依赖后端。

## Phase 2：链与 Pool 数据层

- Chain Registry。
- RPC。
- 区块监听。
- Pool 发现。
- Token 注册。
- Pool 状态更新。
- 历史数据存储。

## Phase 3：交易图与搜索引擎

- Token 图。
- Pool 多重边。
- 路线搜索。
- 闭环识别。
- 路线去重。
- 搜索预算。
- 搜索性能测试。

## Phase 4：报价与优化引擎

- V2。
- V3。
- StableSwap。
- Weighted Pool。
- 多跳计算。
- 输入金额优化。
- Gas 估算。

## Phase 5：模拟与历史审计

- M0–M3。
- 历史状态回放。
- 不同资金规模审计。
- 不同跳数审计。
- 模拟误差验证。

## Phase 6：实时扫描

- WebSocket 监听。
- Pool 增量更新。
- 路线增量搜索。
- 机会排序。
- 机会过期。
- 系统监控。

## Phase 7：Paper Trading

- 模拟资金账户。
- 模拟交易。
- 失败场景。
- 账本。
- 收益统计。
- 风险熔断。

## Phase 8：未来执行模块

本阶段不属于本次扫描引擎的首期交付。

只有在历史审计、Paper Trading 和合约安全验证完成后，才单独设计执行合约和实盘模块。

---

# 第十七章：质量要求与验收标准

## 17.1 搜索正确性

- 同一 Pool 不得被错误识别为多个独立 Pool。
- 同一 Chain ID 内的路线才能组成候选闭环。
- 起始资产与最终资产必须一致。
- 不允许未知 Token。
- 路线去重必须稳定。
- 不允许重复使用 Pool 导致错误报价。
- 所有 AMM 必须使用对应报价模型。

## 17.2 报价正确性

- 多跳输入输出连续。
- 精度处理正确。
- 手续费计算正确。
- V3 Tick 跨越处理正确。
- StableSwap 参数读取正确。
- Weighted Pool 权重计算正确。
- 计算结果与协议官方 Quoter 或链上调用进行交叉验证。

## 17.3 模拟正确性

- 模拟区块状态可复现。
- 同一交易参数重复模拟结果一致。
- 模拟结果记录完整。
- 失败交易记录原因。
- Gas 估算误差可统计。
- 报价与模拟差异可追踪。

## 17.4 性能要求

必须进行不同规模的压力测试：

- 1,000 Pools。
- 10,000 Pools。
- 50,000 Pools。
- 100,000 Pools。

测试指标：

- Pool 状态更新速度。
- 图更新速度。
- 候选路线搜索速度。
- 路线报价速度。
- 模拟队列处理能力。
- 内存占用。
- CPU 占用。

不应在未经实测的情况下承诺固定扫描延迟或机会覆盖率。

## 17.5 资金安全要求

- Paper 账户与未来 Live 账户完全隔离。
- 扫描模块不得拥有私钥。
- 搜索结果不能直接触发链上转账。
- 所有模拟任务必须有明确输入参数。
- 所有数据必须可追踪。
- 任何未经审核的 Pool 和 Token 不得进入高可信机会列表。

## 17.6 最终验收

系统必须至少能够：

1. 发现指定链上的多个 DEX Pool。
2. 构建 Token—Pool 多重图。
3. 搜索双池套利。
4. 搜索三角套利。
5. 搜索四跳、五跳闭环。
6. 搜索跨协议混合 AMM 路线。
7. 根据不同资金规模计算实际回款。
8. 优化投入金额。
9. 估算 Gas。
10. 执行本地 EVM 模拟。
11. 展示完整机会路径。
12. 执行历史数据审计。
13. 执行实时增量扫描。
14. 执行 Paper Trading。
15. 记录全部模拟交易和风险事件。

只有通过上述验收，才能认定 DEX 多池多跳套利扫描引擎的第一阶段研发完成。

---

# 第十八章：重要设计原则

1. 不以理论价格差代替实际可成交利润。
2. 不以现货报价代替多跳整数精度计算。
3. 不以单个 Pool 的流动性代替整条路线的可执行容量。
4. 不以扫描到机会代替模拟通过。
5. 不以模拟通过代替真实链上执行成功。
6. 不以历史收益代替未来收益承诺。
7. 不允许为了提高机会数量而放宽 Token 和 Pool 安全检查。
8. 不允许无限扩大搜索跳数而不考虑计算预算。
9. 不允许把不同区块状态混合成一个报价快照。
10. 不允许将闪电贷本金计入利润。
11. 不允许把交易回滚描述为零成本失败。
12. 不允许将未经验证的 EVM 网络直接标记为可实盘。
13. 扫描引擎与执行引擎必须保持模块隔离。
14. 所有机会必须能够追溯其 Pool、区块、报价、计算过程和模拟结果。
15. 任何策略是否值得进入实盘，都必须由历史审计、Paper Trading 和安全审核提供证据。

**最终产品目标：建立一个可扩展、可审计、可复现、支持多协议和多跳闭环的 DEX 原子套利搜索引擎，而不是一个只会显示两个池子价格差的简单扫描器。**