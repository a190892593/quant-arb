import { NAV } from "../components/nav";

/** 未知路由的兜底页：列出全部已实现视图，避免白屏 */
export function Pending({ route }: { route: string }) {
  return (
    <div className="stack">
      <div className="page-head">
        <div className="page-head-main">
          <div className="page-eyebrow">NOT FOUND</div>
          <h1>未找到该页面</h1>
          <p className="page-sub">
            路由 <code className="mono">{route || "（空）"}</code> 不存在。以下视图均已实现：
          </p>
        </div>
      </div>
      <section className="panel">
        <div className="panel-body">
          <div className="ev-list">
            {NAV.flatMap((g) => g.items).map((item) => (
              <div className="ev" key={item.id}>
                <div className="ev-top">
                  <a className="mono" href={item.hash}>
                    {item.label}
                  </a>
                  <span className="ev-time">{item.hash}</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      </section>
    </div>
  );
}
