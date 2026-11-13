import { useEffect, useState, type ReactNode } from "react";
import { api, errorText } from "../../shared/api/api";
import { currentLocale, localizedPath } from "../../shared/i18n/locale";

export function conversationResource(href: string | undefined, endpoint: string | undefined) {
  const workspace = endpoint?.match(/^\/workspaces\/([^/]+)\/conversations\//)?.[1];
  if (!workspace || !href) return null;
  let url: URL;
  try { url = new URL(href, location.origin); } catch { return null; }
  if (url.origin !== location.origin || url.hash) return null;
  const artifacts = url.pathname.match(/^\/api\/workspaces\/([^/]+)\/pipeline-artifacts\/?$/);
  const run = url.searchParams.get("run_id");
  if (artifacts?.[1] === workspace && run && /^\d+$/.test(run) && [...url.searchParams].length === 1) {
    return { workspace, kind: "pipeline-artifacts-list", id: run };
  }
  if (url.search) return null;
  const match = url.pathname.match(/^\/api\/workspaces\/([^/]+)\/(workflows|workflow-executions|pipelines|pipeline-runs|pipeline-artifacts)\/(\d+)(\/content)?\/?$/);
  if (match && (match[2] === "pipeline-artifacts") !== Boolean(match[4])) return null;
  return match?.[1] === workspace ? { workspace, kind: match[2]!, id: match[3]! } : null;
}

/** API URLs are data endpoints. Resolve executions through their authorized parent. */
export function ConversationResourceLink({ resource, children }: { resource: NonNullable<ReturnType<typeof conversationResource>>; children: ReactNode }) {
  const { workspace, kind, id } = resource;
  const [parent, setParent] = useState("");
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (kind !== "workflow-executions") return;
    const controller = new AbortController(); setParent(""); setError("");
    void api<{ workflow_id: string }>(`/workspaces/${workspace}/workflow-executions/${id}`, { signal: controller.signal })
      .then(row => { if (!controller.signal.aborted && /^\d+$/.test(row.workflow_id)) setParent(row.workflow_id); })
      .catch(cause => { if (!controller.signal.aborted) setError(errorText(cause)); });
    return () => controller.abort();
  }, [workspace, kind, id, retry]);
  if (kind === "pipeline-artifacts-list") {
    const url = localizedPath(`/workspaces/${workspace}/pipeline-artifacts`, currentLocale()) + `?run_id=${id}`;
    return <a href={url} target="_blank" rel="noopener noreferrer">{children}</a>;
  }
  if (kind === "pipeline-artifacts") return <a href={`/api/workspaces/${workspace}/pipeline-artifacts/${id}/content`} download>{children}</a>;
  if (kind === "pipelines" || kind === "pipeline-runs") {
    const url = localizedPath(`/workspaces/${workspace}/${kind}/${id}`, currentLocale());
    return <a href={url} target="_blank" rel="noopener noreferrer">{children}</a>;
  }
  const workflow = kind === "workflows" ? id : parent;
  if (!workflow) return <span><button type="button" disabled={!error} aria-busy={!error} title={error || undefined} className="text-primary underline underline-offset-4 disabled:cursor-wait" onClick={() => setRetry(v => v + 1)}>{children}</button>{error && <span role="alert" className="ml-2 text-xs text-destructive">{error}</span>}</span>;
  const url = localizedPath(`/workspaces/${workspace}/workflows/${workflow}`, currentLocale()) + (kind === "workflow-executions" ? `?execution=${id}` : "");
  return <a href={url} target="_blank" rel="noopener noreferrer">{children}</a>;
}
