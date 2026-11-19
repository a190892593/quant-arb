//! ARBX 后端进程入口。
//!
//! 用法：
//! ```text
//! arb-backend             # 启动 API + 扫描器（默认）
//! arb-backend serve       # 同上
//! arb-backend check       # 只做配置/数据库/账本自检后退出
//! arb-backend audit       # 独立审计 CLI（读取已记录的 Paper 数据并输出报告）
//! arb-backend migrate     # 只执行数据库迁移
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use arb_backend::config::Config;
use arb_backend::db::Db;
use arb_backend::engine::{self, AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("serve");

    let workspace = resolve_workspace()?;
    let cfg = Config::load(&workspace)?;
    std::fs::create_dir_all(cfg.data_dir(&workspace))?;

    let db = Db::open(&cfg.db_path(&workspace))?;
    tracing::info!("数据库就绪: {}", db.path());

    match cmd {
        "migrate" => {
            println!("数据库迁移完成: {}", db.path());
            return Ok(());
        }
        "check" => {
            let state = AppState::new(cfg.clone(), workspace.clone(), db)?;
            engine::seed::restore(&state).await?;
            engine::seed::seed_inventory(&state).await?;
            let health = engine::scanner::health_json(&state).await;
            let ledger = state.ledger.lock().await;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "workspace": workspace.display().to_string(),
                    "config_ok": true,
                    "health": health,
                    "accounts": ledger.accounts().len(),
                    "equity": ledger.total_equity(),
                    "start_equity": ledger.start_equity(),
                    "allocation": engine::seed::allocation_note(
                        cfg.paper.start_equity, cfg.paper.dex_share
                    ),
                }))?
            );
            return Ok(());
        }
        "audit" => {
            let state = AppState::new(cfg.clone(), workspace.clone(), db)?;
            engine::seed::restore(&state).await?;
            let report = audit_cli(&state).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        "backup" => {
            // 一致性备份：SQLite 的 VACUUM INTO 会生成一份已整理的事务一致副本
            let out = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| cfg.data_dir(&workspace).join("arbx-backup.db"));
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let target = out.display().to_string();
            db.with(|c| {
                c.execute("VACUUM INTO ?1", rusqlite::params![target])?;
                Ok(())
            })?;
            println!("备份完成: {}", out.display());
            return Ok(());
        }
        "serve" | _ => {}
    }

    let state = AppState::new(cfg.clone(), workspace.clone(), db)?;
    engine::seed::restore(&state).await?;
    engine::seed::seed_inventory(&state).await?;

    // 启动扫描任务（默认不自动 RUNNING，需要操作者显式启动）
    let _scanner = engine::scanner::spawn(state.clone());

    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(
        "ARBX 后端已启动 http://{addr}  (UI: {}, feed: {}, {})",
        cfg.ui_dir(&workspace).display(),
        cfg.feed.mode,
        engine::seed::allocation_note(cfg.paper.start_equity, cfg.paper.dex_share)
    );
    tracing::info!("第一阶段为 Paper 模式：不广播链上交易，不提交真实 CEX 订单");

    let app = arb_backend::api::router(state.clone());
    axum::serve(listener, app).await?;
    Ok(())
}

fn init_tracing() {
    let filter = std::env::var("ARBX_LOG")
        .unwrap_or_else(|_| "info,arb_backend=info,tower_http=warn".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&filter)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();
}

/// 工作区根目录：优先环境变量，其次当前目录，最后可执行文件所在目录的上一级。
fn resolve_workspace() -> anyhow::Result<PathBuf> {
    if let Ok(p) = std::env::var("ARBX_WORKSPACE") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Ok(p);
        }
    }
    let cwd = std::env::current_dir()?;
    // 当前目录或其父目录包含 config/ 或 web/ 即认为是工作区
    for candidate in [cwd.clone(), cwd.join("..")] {
        if candidate.join("config").exists() || candidate.join("web").exists() {
            return Ok(candidate.canonicalize().unwrap_or(candidate));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [dir.to_path_buf(), dir.join(".."), dir.join("..").join("..")] {
                if candidate.join("web").exists() || candidate.join("config").exists() {
                    return Ok(candidate.canonicalize().unwrap_or(candidate));
                }
            }
        }
    }
    Ok(cwd)
}

/// 独立审计 CLI：只统计已记录数据，不补造缺失区间。
async fn audit_cli(state: &Arc<AppState>) -> anyhow::Result<serde_json::Value> {
    let execs = state.executions.read().await.clone();
    let ledger = state.ledger.lock().await;
    let n = execs.len();
    let wins = execs.iter().filter(|e| e.net_pnl > rust_decimal::Decimal::ZERO).count();
    let reverts = execs
        .iter()
        .filter(|e| {
            matches!(
                e.state,
                arb_backend::domain::types::ExecutionState::DexReverted
                    | arb_backend::domain::types::ExecutionState::HedgeFailed
            )
        })
        .count();
    let net: rust_decimal::Decimal = execs.iter().map(|e| e.net_pnl).sum();
    let cost: rust_decimal::Decimal = execs.iter().map(|e| e.costs.total()).sum();
    let verdict = if n < 20 {
        "WATCH"
    } else if net <= rust_decimal::Decimal::ZERO {
        "REJECT"
    } else if reverts * 5 > n {
        "PAPER"
    } else {
        "PASS"
    };
    Ok(serde_json::json!({
        "generated_at": chrono::Utc::now(),
        "executions": n,
        "wins": wins,
        "reverts": reverts,
        "net_pnl": net,
        "total_cost": cost,
        "equity": ledger.total_equity(),
        "start_equity": ledger.start_equity(),
        "verdict": verdict,
        "caveats": [
            "仅覆盖本机已记录数据",
            "缺少历史 L2 盘口的区间不得作为高可信结论",
            "结论不代表未来实盘收益"
        ]
    }))
}
