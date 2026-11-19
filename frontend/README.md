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
