import { lazy, Suspense, useEffect, useState } from "react";
import { ArrowLeft, Columns2, FolderOpen, Plus, Rows2, SquareTerminal, X } from "lucide-react";
import { api, errorText } from "../api";
import { href } from "../router";
import { Button } from "./ui/button";
import { HostFiles } from "./HostFiles";

const HostTerminal = lazy(() => import("./HostTerminal").then(module => ({ default: module.HostTerminal })));

type Kind = "terminal" | "files";
type Pane = { id: string; kind: Kind; directory?: string };
type Node = Pane | { id: string; axis: "horizontal" | "vertical"; children: [Node, Node] };
type WorkspaceTab = { id: string; root: Node; activePane: string; label: string };
let nextId = 0;
const id = () => `pane-${++nextId}`;
const pane = (kind: Kind, directory?: string): Pane => ({ id: id(), kind, ...(directory ? { directory } : {}) });
const replace = (node: Node, target: string, update: (current: Pane) => Node): Node => {
  if (!("children" in node)) return node.id === target ? update(node) : node;
  return { ...node, children: node.children.map(child => replace(child, target, update)) as [Node, Node] };
};
const remove = (node: Node, target: string): Node | null => {
  if (!("children" in node)) return node.id === target ? null : node;
  const children = node.children.map(child => remove(child, target));
  return children[0] && children[1] ? { ...node, children: children as [Node, Node] } : children[0] ?? children[1] ?? null;
};
const firstPane = (node: Node): Pane => "children" in node ? firstPane(node.children[0]) : node;

export function HostWorkspace({ workspace, hostId, permissions }: { workspace: string; hostId: string; permissions: Set<string> }) {
  const canTerminal = permissions.has("hosts.terminal");
  const canFiles = permissions.has("hosts.files");
  const initialKind: Kind = new URLSearchParams(location.search).get("tool") === "files" && canFiles ? "files" : canTerminal ? "terminal" : "files";
  const initialDirectory = new URLSearchParams(location.search).get("directory") ?? undefined;
  const [tabs, setTabs] = useState<WorkspaceTab[]>(() => {
    const root = pane(initialKind, initialDirectory);
    return [{ id: id(), root, activePane: root.id, label: initialKind === "terminal" ? "终端 1" : "文件 1" }];
  });
  const [activeTab, setActiveTab] = useState(tabs[0]!.id);
  const [hostName, setHostName] = useState("主机工作台");
  const [error, setError] = useState("");

  useEffect(() => {
    const controller = new AbortController();
    void api<{ name: string }>(`/workspaces/${encodeURIComponent(workspace)}/hosts/${encodeURIComponent(hostId)}`, { signal: controller.signal })
      .then(host => { if (!controller.signal.aborted) setHostName(host.name); })
      .catch(cause => { if (!controller.signal.aborted) setError(errorText(cause)); });
    return () => controller.abort();
  }, [workspace, hostId]);

  const addTab = (kind: Kind, directory?: string) => {
    const root = pane(kind, directory);
    const tab = { id: id(), root, activePane: root.id, label: `${kind === "terminal" ? "终端" : "文件"} ${tabs.filter(item => firstPane(item.root).kind === kind).length + 1}` };
    setTabs(current => [...current, tab]);
    setActiveTab(tab.id);
  };
  const changeTab = (update: (tab: WorkspaceTab) => WorkspaceTab) => setTabs(current => current.map(tab => tab.id === activeTab ? update(tab) : tab));
  const split = (target: string, axis: "horizontal" | "vertical", kind: Kind, directory?: string) => {
    const sibling = pane(kind, directory);
    changeTab(tab => ({ ...tab, activePane: sibling.id, root: replace(tab.root, target, current => ({ id: id(), axis, children: [current, sibling] })) }));
  };
  const closePane = (target: string) => changeTab(tab => {
    const root = remove(tab.root, target) ?? tab.root;
    return { ...tab, root, activePane: firstPane(root).id };
  });
  const closeTab = (target: string) => {
    const remaining = tabs.filter(tab => tab.id !== target);
    if (!remaining.length) {
      const root = pane(canTerminal ? "terminal" : "files");
      const replacement = { id: id(), root, activePane: root.id, label: canTerminal ? "终端 1" : "文件 1" };
      setTabs([replacement]); setActiveTab(replacement.id); return;
    }
    setTabs(remaining);
    if (activeTab === target) setActiveTab(remaining.at(-1)!.id);
  };

  const renderNode = (node: Node, tab: WorkspaceTab, canClose: boolean): React.ReactNode => {
    if ("children" in node) return <div className={`block min-w-0 md:flex md:min-h-0 md:flex-1 ${node.axis === "horizontal" ? "md:flex-row" : "md:flex-col"}`}>
      {node.children.map((child, index) => <div key={child.id} className={`flex min-w-0 w-full md:h-auto md:min-h-0 md:flex-1 ${"children" in child ? "" : "h-[420px]"} ${index === 0 ? node.axis === "horizontal" ? "border-b md:border-r md:border-b-0" : "border-b" : ""}`}>{renderNode(child, tab, true)}</div>)}
    </div>;
    return <section className={`flex h-full min-h-[420px] min-w-0 flex-1 flex-col md:min-h-0 ${tab.activePane === node.id ? "bg-accent/20" : ""}`} onPointerDown={() => { if (tab.id === activeTab) changeTab(current => ({ ...current, activePane: node.id })); }}>
      <div className="flex h-10 shrink-0 items-center gap-1 border-b px-2">
        {node.kind === "terminal" ? <SquareTerminal className="size-4 shrink-0" /> : <FolderOpen className="size-4 shrink-0" />}
        <span className="min-w-0 flex-1 truncate text-sm" title={node.directory}>{node.directory || (node.kind === "terminal" ? "终端" : "文件")}</span>
        {canTerminal && <Button variant="ghost" size="icon" className="size-7" aria-label="切换为终端" title="终端" disabled={node.kind === "terminal"} onClick={() => changeTab(current => ({ ...current, root: replace(current.root, node.id, old => ({ ...old, kind: "terminal" })) }))}><SquareTerminal className="size-4" /></Button>}
        {canFiles && <Button variant="ghost" size="icon" className="size-7" aria-label="切换为文件" title="文件" disabled={node.kind === "files"} onClick={() => changeTab(current => ({ ...current, root: replace(current.root, node.id, old => ({ id: old.id, kind: "files" })) }))}><FolderOpen className="size-4" /></Button>}
        <Button variant="ghost" size="icon" className="size-7" aria-label="左右分屏" title="左右分屏" onClick={() => split(node.id, "horizontal", node.kind)}><Columns2 className="size-4" /></Button>
        <Button variant="ghost" size="icon" className="size-7" aria-label="上下分屏" title="上下分屏" onClick={() => split(node.id, "vertical", node.kind)}><Rows2 className="size-4" /></Button>
        {canClose && <Button variant="ghost" size="icon" className="size-7" aria-label="关闭分屏" title="关闭分屏" onClick={() => closePane(node.id)}><X className="size-4" /></Button>}
      </div>
      <div className="min-h-0 min-w-0 flex-1 p-2">
        {node.kind === "terminal" ? <Suspense fallback={<p role="status">正在加载终端…</p>}><HostTerminal workspace={workspace} hostId={hostId} {...(node.directory ? { directory: node.directory } : {})} /></Suspense> :
          <HostFiles workspace={workspace} hostId={hostId} canTerminal={canTerminal} {...(node.directory ? { initialPath: node.directory } : {})} onOpenTerminal={directory => split(node.id, "horizontal", "terminal", directory)} />}
      </div>
    </section>;
  };

  if (!canTerminal && !canFiles) return <p role="alert" className="p-6">当前角色没有终端或文件权限。</p>;
  return <div className="flex h-dvh min-h-[420px] min-w-0 flex-col bg-background text-foreground">
    <header className="flex h-12 shrink-0 items-center gap-3 border-b px-3">
      <a href={href({ workspace, group: "host", item: "hosts", id: hostId, mode: "view" }, {})} className="inline-flex size-8 items-center justify-center rounded hover:bg-accent" aria-label="返回主机详情" title="返回主机详情"><ArrowLeft className="size-4" /></a>
      <h1 className="min-w-0 flex-1 truncate text-sm font-semibold" title={hostName}>{hostName}</h1>
      {canTerminal && <Button variant="outline" size="sm" onClick={() => addTab("terminal")}><SquareTerminal className="size-4" />新建终端</Button>}
      {canFiles && <Button variant="outline" size="sm" onClick={() => addTab("files")}><FolderOpen className="size-4" />新建文件</Button>}
    </header>
    {error && <p role="alert" className="border-b px-3 py-1 text-sm text-destructive">{error}</p>}
    <div role="tablist" aria-label="主机工作标签" className="flex h-10 shrink-0 items-center gap-1 overflow-x-auto border-b px-2">
      {tabs.map(tab => <div key={tab.id} className={`inline-flex h-8 shrink-0 items-center rounded border ${activeTab === tab.id ? "bg-accent" : ""}`}>
        <button type="button" role="tab" aria-selected={activeTab === tab.id} className="h-full px-3 text-sm" onClick={() => setActiveTab(tab.id)}>{tab.label}</button>
        <Button variant="ghost" size="icon" className="size-7" aria-label={`关闭${tab.label}`} onClick={() => closeTab(tab.id)}><X className="size-3.5" /></Button>
      </div>)}
      <Button variant="ghost" size="icon" className="size-7 shrink-0" aria-label="新建标签" title="新建标签" onClick={() => addTab(canTerminal ? "terminal" : "files")}><Plus className="size-4" /></Button>
    </div>
    {tabs.map(tab => <div key={tab.id} role="tabpanel" className={activeTab === tab.id ? "block min-h-0 min-w-0 flex-1 overflow-auto md:flex md:overflow-hidden" : "hidden"}>{renderNode(tab.root, tab, false)}</div>)}
  </div>;
}
