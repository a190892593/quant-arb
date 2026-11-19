# ARBX 前端（React + TypeScript）

TDD §1.2 / §2.2 规定的前端形态：**React 19 + TypeScript + Vite**，本地开发服务 `arb-ui`。
原 `web/` 下的原生 JS 原型已全部迁移完成并移除，样式文件原样迁入 `src/styles/`。

## 启动

```powershell
cd frontend
npm install
npm run dev        # http://localhost:5173
npm run build      # tsc -b && vite build → dist/
npm run typecheck  # tsc --noEmit
```

> Windows 上若 `vite build` 报 esbuild `Access is denied`（临时目录权限），把 TEMP/TMP 指向工作区内再构建：
> `$env:TEMP="D:\VibeCoding\quant-arb\.build\tmp"; $env:TMP=$env:TEMP; npm run build`

## 覆盖度

| 模块 | 状态 |
|---|---|
| 工程骨架（Vite 7 + React 19 + TS strict） | ✅ |
| 设计令牌与全部样式 | ✅ |
| 类型层 `src/types.ts`（领域模型 + 全部集合） | ✅ |
| 数据层 `src/data/dataset.ts`（演示数据集，已 TS 化） | ✅ |
| 数据边界 `src/data/index.ts`（接 API 只改这里） | ✅ |
| 外壳：顶栏、导航、风险脊、hash 路由 | ✅ |
| 图表：`LineChart`（悬停/回撤填充）、`Spark`、`Waterfall` | ✅ |
| 机会抽屉（5 页签 + 倒计时 + Esc） | ✅ |
| 扫描活动条 `ScanStrip`（光束 / 对象轮换 / 事件轮播） | ✅ |
| 命令面板（Ctrl/Cmd + K） | ✅ |
| 总览 / DEX 扫描 / CEX–DEX 扫描 | ✅ |
| Paper 执行 / 账户与库存 / 绩效 | ✅ |
| 账本 / 风控台 / 执行合约 / 系统 / 能力矩阵 | ✅ |
| 策略配置 ×3（三角 / 跨池 / 对冲） | ✅ |
| 池与 Token / 历史审计 / 数据质量 | ✅ |

17 条路由全部由 React 组件渲染，`tsc --noEmit` 在 strict + noUnusedLocals 下 0 报错。

## 目录

```
src/
├── main.tsx              入口
├── App.tsx               路由表 + 布局 + 抽屉/命令面板挂载
├── types.ts              领域模型（与后端契约同名）
├── data/
│   ├── dataset.ts        演示数据集（确定性生成，纯函数）
│   ├── index.ts          ★ 唯一数据边界
│   └── store.ts          扫描开关共享状态
├── lib/
│   ├── format.ts         金额 / 比例 / 年龄格式化
│   ├── icons.tsx         16px 线性图标集
│   ├── opportunity.ts    路线 / 容量 / 失效 / 评分 / 闸门 计算
│   └── scanEvents.ts     扫描事件文案
├── charts/               LineChart / Spark / Waterfall（手写 SVG）
├── ui/                   Chip / StatusChip / QualityChip / Toast
├── components/           Shell、OppTable、OppDrawer、ScanStrip、CommandPalette
├── views/                16 个页面组件
└── styles/               tokens / app / views
```

## 约定

1. **数据只走 `src/data`**：页面不直接引用 `dataset.ts`。
2. **类型先行**：新增字段先改 `types.ts`，再改组件；`npm run typecheck` 必须为 0。
3. **样式不改**：`styles/` 与原型逐字节一致，类名即组件契约。
4. **一个视图一个文件**，与路由表一一对应。

## 与后端对接

后端为 Axum 0.8 + WebSocket（`backend/src/api`）。接入时只需替换 `src/data/index.ts`：

- `GET /opportunities`、`GET /opportunities/{id}` → `data.opps`
- `GET /scanner/status`、`POST /scanner/start|stop` → `scanStore`
- `GET /risk/status`、`GET /risk/events` → 风控台
- `GET /ledger`、`GET /paper/*` → 账本与执行
- WS `opportunity.new` / `risk.event` / `paper.order` → 推送到 store，组件被动重渲染

页面组件不感知数据来源，所以接口就绪后不需要改视图。

---

## 联调状态（对接 `backend/` Axum API）

前端已接上本地后端，页面显示的是 Paper 账本的真实数据，不再是硬编码演示值。

**运行方式**

```powershell
cd frontend
npm run build      # 产出 dist/
npm run publish    # dist/ → 仓库根 ui/（后端 ui_dir 指向这里）
cd ..; .\scripts\start.ps1 -Open
```

后端 `config/default.toml` 的 `ui_dir = "ui"`，`http://127.0.0.1:8791` 同时提供 API 与前端。
开发时可用 `npm run dev`（Vite 5173），需要把 `/api` 代理到 8791（见 `vite.config.ts`）。

**接口映射**（`src/data/api.ts`）

| 前端集合 | 后端接口 |
|---|---|
| `scanner` | `GET /api/scanner/status` |
| `opps` | `GET /api/opportunities`（`dex_route.legs` / `costs.items` / `risk_score.components` 展开） |
| `executions` | `GET /api/executions` |
| `series` / `equity` / `startEquity` | `GET /api/paper/performance` |
| `venues` | `GET /api/paper/accounts`（按 venue 聚合余额） |
| `ledger` / `ledgerAccounts` / `flashLoans` | `GET /api/ledger` |
| `instruments` / `cexIntegrity` / `health` | `GET /api/markets/data-health` + `GET /api/exchanges/{id}/instruments` |
| `riskStatus` / `riskParams` / `riskGroups` | `GET /api/risk/status`、`/api/risk/params` |
| `circuitRules` | `GET /api/risk/rules` |
| `pools` / `tokens` / `adapters` | `GET /api/dex/pools`、`/api/tokens`、`/api/dex/protocols` |
| `chains` / `services` | `GET /api/chains`、`/api/system/status` |
| `auditJobs` | `GET /api/audit/jobs` |
| `contractComponents` / `Tests` / `Events` | `GET /api/contract/status` |

**轮询**：`startPolling()` 默认 3 s 一次全量快照，页面隐藏时暂停；失败时保留最后一份有效数据并在顶栏标记离线。

**写操作**（`src/data/index.ts` 的 `actions`，唯一会改变后端状态的地方）

| 动作 | 接口 |
|---|---|
| 扫描启停（两页共享一个引擎） | `POST /api/scanner/start` / `stop` |
| 全局暂停 / 恢复 | `POST /api/risk/pause` / `resume` |
| 生成并执行 Paper 计划 | `POST /api/opportunities/{id}/execute` |
| 重置 Paper 账户 | `POST /api/paper/reset` |
| 发起历史审计 | `POST /api/audit/jobs` |

**已知限制**

- 后端 demo feed 只产出 DEX 原子机会，`CEX–DEX 扫描` 当前为 0 条（页面显示空状态，不是故障）。
- 拒绝原因在表格里截断为一行，完整文本在行悬停 `title` 中。
- 重启后端后扫描器为 `STOPPED`，需在页面点「启动扫描」或调 `/api/scanner/start`。