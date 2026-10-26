import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Button } from "@/components/ui/button";

interface Summary {
  counts: Record<string, number>;
  states: Record<string, Record<string, number>>;
  failures_24h: Record<string, number>;
  runners:
    | {
        id: string;
        name: string;
        seen_at: number | null;
        lag_ms: number | null;
      }[]
    | null;
  recent:
    | {
        id: string;
        actor_name: string;
        action: string;
        resource: string;
        created_at: number;
      }[]
    | null;
  as_of: number;
}
const labels: Record<string, string> = {
  hosts: "主机",
  "host-groups": "主机组",
  tags: "标签",
  credentials: "授权认证",
  repositories: "仓库",
  pipelines: "流水线",
  deployments: "应用部署",
  members: "成员",
  roles: "角色",
  "pipeline-runs": "流水线运行",
  "deployment-revisions": "部署执行",
  online: "在线",
  offline: "离线",
  unknown: "未知",
  pending: "待执行",
  queued: "排队中",
  running: "运行中",
  succeeded: "成功",
  failed: "失败",
  cancelled: "已取消",
  idle: "未部署",
  create: "创建",
  update: "修改",
  delete: "删除",
  "rotate-invitation": "更新邀请码",
};
const label = (key: string) => labels[key] ?? key;
export function Dashboard({ workspace }: { workspace: string }) {
  const [data, setData] = useState<Summary>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(true);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setBusy(true);
    setError("");
    void api<Summary>(`/workspaces/${workspace}/dashboard`, {
      signal: controller.signal,
    })
      .then(setData)
      .catch((cause) => {
        if (!controller.signal.aborted) setError(errorText(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setBusy(false);
      });
    return () => controller.abort();
  }, [workspace, attempt]);
  return (
    <div className="min-h-0 space-y-5 overflow-auto pb-6">
      <div className="flex items-center gap-3">
        <Button
          variant="outline"
          disabled={busy}
          onClick={() => setAttempt((n) => n + 1)}
        >
          {busy ? "载入中…" : "刷新"}
        </Button>
        {data && (
          <span className="text-xs text-muted-foreground">
            更新于 {new Date(data.as_of).toLocaleString()}
          </span>
        )}
      </div>
      {error && (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      )}
      {data && (
        <>
          {!Object.keys(data.counts).length && (
            <p className="text-sm text-muted-foreground">
              当前角色没有可查看的资源统计。
            </p>
          )}
          <section
            aria-label="资源概览"
            className="grid grid-cols-2 gap-3 lg:grid-cols-4"
          >
            {Object.entries(data.counts).map(([key, count]) => (
              <div key={key} className="rounded-md border p-4">
                <h2 className="text-sm text-muted-foreground">{label(key)}</h2>
                <p className="mt-2 text-2xl font-semibold">{count}</p>
              </div>
            ))}
          </section>
          <div className="grid gap-4 lg:grid-cols-2">
            {Object.entries(data.states).map(([key, states]) => (
              <section key={key} className="rounded-md border p-4">
                <h2 className="mb-3 font-medium">{label(key)}状态</h2>
                {Object.keys(states).length ? (
                  <dl className="flex flex-wrap gap-5">
                    {Object.entries(states).map(([state, count]) => (
                      <div key={state}>
                        <dt className="text-sm text-muted-foreground">
                          {label(state)}
                        </dt>
                        <dd className="text-xl">{count}</dd>
                      </div>
                    ))}
                  </dl>
                ) : (
                  <p className="text-sm text-muted-foreground">暂无记录</p>
                )}
              </section>
            ))}
          </div>
          {!!Object.keys(data.failures_24h).length && (
            <section className="rounded-md border p-4">
              <h2 className="mb-3 font-medium">最近 24 小时创建且失败的执行</h2>
              <dl className="flex gap-6">
                {Object.entries(data.failures_24h).map(([key, value]) => (
                  <div key={key}>
                    <dt className="text-sm text-muted-foreground">
                      {label(key)}
                    </dt>
                    <dd className="text-xl">{value}</dd>
                  </div>
                ))}
              </dl>
            </section>
          )}
          {data.runners && (
            <section className="rounded-md border p-4">
              <h2 className="mb-3 font-medium">
                Runner 心跳 · 最久未上报的 10 台
              </h2>
              {data.runners.length ? (
                <ul className="space-y-2">
                  {data.runners.map((r) => (
                    <li
                      key={r.id}
                      className="flex justify-between gap-4 text-sm"
                    >
                      <span>{r.name}</span>
                      <span className="text-muted-foreground">
                        {r.lag_ms === null
                          ? "尚未上报"
                          : `${Math.floor(r.lag_ms / 1000)} 秒前`}
                      </span>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-sm text-muted-foreground">
                  暂无 Runner 主机
                </p>
              )}
            </section>
          )}
          {data.recent && (
            <section className="rounded-md border p-4">
              <h2 className="mb-3 font-medium">近期操作</h2>
              {data.recent.length ? (
                <ul className="space-y-2">
                  {data.recent.map((a) => (
                    <li
                      key={a.id}
                      className="flex flex-wrap justify-between gap-2 text-sm"
                    >
                      <span>
                        {a.actor_name} · {label(a.action)} · {label(a.resource)}
                      </span>
                      <time className="text-muted-foreground">
                        {new Date(a.created_at).toLocaleString()}
                      </time>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="text-sm text-muted-foreground">暂无操作记录</p>
              )}
            </section>
          )}
        </>
      )}
    </div>
  );
}
