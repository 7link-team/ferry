import { useEffect, useState } from "react";
import { api, errorText, type Status } from "./api";
import { nav, findGroup, findItem } from "./nav";
import { href, navigate, useRoute } from "./router";

/** Top module bar, left menu, breadcrumbs and page title — the shell every
 *  resource page renders inside. Pages supply content only. */
export function App() {
  const route = useRoute();
  const [status, setStatus] = useState<Status>();
  const [error, setError] = useState("");
  useEffect(() => {
    api<Status>("/status").then(setStatus, (e: unknown) => setError(errorText(e)));
  }, []);
  // ponytail: the workspace comes from the URL until sessions land.
  const workspace = route.workspace || "demo";
  const group = findGroup(route.group)!;
  const item = findItem(route.group, route.item)!;

  return (
    <div className="shell">
      <header className="topbar">
        <a className="brand" href={href({ workspace, group: nav[0]!.key, item: nav[0]!.items[0]!.key, mode: "list" })}
           onClick={link({ workspace, group: nav[0]!.key, item: nav[0]!.items[0]!.key })}>
          Ferry
        </a>
        <nav className="modules" aria-label="模块">
          {nav.map((g) => (
            <a key={g.key}
               href={href({ workspace, group: g.key, item: g.items[0]!.key, mode: "list" })}
               aria-current={g.key === group.key ? "page" : undefined}
               onClick={link({ workspace, group: g.key, item: g.items[0]!.key })}>
              {g.label}
            </a>
          ))}
        </nav>
        <span className="spacer" />
        <span className="workspace">{workspace}</span>
      </header>

      <div className="body">
        <nav className="sidemenu" aria-label={group.label}>
          {group.items.map((i) => (
            <a key={i.key}
               href={href({ workspace, group: group.key, item: i.key, mode: "list" }, i.params)}
               aria-current={i.key === item.key ? "page" : undefined}
               onClick={link({ workspace, group: group.key, item: i.key }, i.params)}>
              {i.label}
            </a>
          ))}
        </nav>

        <main className="content">
          <nav className="breadcrumbs" aria-label="面包屑">
            <ol>
              <li><a href={href({ workspace, group: nav[0]!.key, item: nav[0]!.items[0]!.key, mode: "list" })}
                     onClick={link({ workspace, group: nav[0]!.key, item: nav[0]!.items[0]!.key })}>工作空间</a></li>
              <li><a href={href({ workspace, group: group.key, item: group.items[0]!.key, mode: "list" })}
                     onClick={link({ workspace, group: group.key, item: group.items[0]!.key })}>{group.label}</a></li>
              <li>{route.mode === "list"
                ? <span aria-current="page">{item.label}</span>
                : <a href={href({ workspace, group: group.key, item: item.key, mode: "list" })}
                     onClick={link({ workspace, group: group.key, item: item.key })}>{item.label}</a>}</li>
              {route.mode !== "list" && (
                <li><span aria-current="page">
                  {route.mode === "new" ? "创建" : route.mode === "edit" ? "编辑" : "详情"}
                </span></li>
              )}
            </ol>
          </nav>
          <h1 className="page-title">{item.label}</h1>

          <section className="page">
            {error && <p className="notice error">{error}</p>}
            <p className="placeholder">
              资源页 <code>{item.resource}</code>
              {item.params ? `（${new URLSearchParams(item.params)}）` : ""} 还未实现。
              统一列表与表单组件的设计见 <code>docs/ferry-运维平台产品与技术方案.md</code>。
            </p>
            {status && (
              <p className="muted">
                服务 v{status.version}，协议 v{status.protocol}，
                {status.server_mode ? "Docker 部署" : "桌面部署"}
              </p>
            )}
          </section>
        </main>
      </div>
    </div>
  );
}

/** Real links that the router intercepts, so middle-click and copy still work. */
const link =
  (route: { workspace: string; group: string; item: string }, query?: Record<string, string>) =>
  (event: React.MouseEvent) => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    navigate({ ...route, mode: "list" }, query ?? {});
  };
