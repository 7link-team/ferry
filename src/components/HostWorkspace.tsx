import { lazy, Suspense, useEffect, useState, type CSSProperties } from "react";
import { ArrowLeft, Columns2, FolderOpen, Plus, Rows2, SquareTerminal, X } from "lucide-react";
import { api, errorText } from "../api";
import { href } from "../router";
import { Button } from "./ui/button";
import { HostFiles } from "./HostFiles";
import { HostLatency } from "./HostLatency";
import { HostTelemetry } from "./HostTelemetry";
import { Tabs } from "radix-ui";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./ui/dropdown-menu";

const HostTerminal = lazy(() => import("./HostTerminal").then(module => ({ default: module.HostTerminal })));

type Kind = "terminal" | "files";
type Pane = { id: string; kind: Kind; opened: Kind[]; directory?: string };
type Node = Pane | { id: string; axis: "horizontal" | "vertical"; children: [Node, Node] };
type WorkspaceTab = { id: string; root: Node; activePane: string; label: string };
let nextId = 0;
const id = () => `pane-${++nextId}`;
const pane = (kind: Kind, directory?: string): Pane => ({ id: id(), kind, opened: [kind], ...(directory ? { directory } : {}) });
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
const hasTerminal = (node: Node): boolean => "children" in node ? node.children.some(hasTerminal) : node.kind === "terminal";
type Rect = { left: number; top: number; width: number; height: number };
function arrange(node: Node, rect: Rect = { left: 0, top: 0, width: 100, height: 100 }): { node: Pane; rect: Rect }[] {
  if (!("children" in node)) return [{ node, rect }];
  const horizontal = node.axis === "horizontal";
  const half = { ...rect, ...(horizontal ? { width: rect.width / 2 } : { height: rect.height / 2 }) };
  return [...arrange(node.children[0], half), ...arrange(node.children[1], {
    ...half, ...(horizontal ? { left: rect.left + half.width } : { top: rect.top + half.height }),
  })];
}

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
    void api<{ name: string }>(`/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}`, { signal: controller.signal })
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

  const renderPane = (node: Pane, tab: WorkspaceTab, canClose: boolean): React.ReactNode => {
    return <Tabs.Root value={node.kind} onValueChange={value => changeTab(current => ({ ...current, root: replace(current.root, node.id, old => ({ ...old, kind: value as Kind, opened: [...new Set([...old.opened, value as Kind])] })) }))} asChild><section aria-label={node.kind === "terminal" ? "终端分屏" : "文件分屏"} className={`flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden ${tab.activePane === node.id ? "bg-accent/20" : ""}`} onPointerDown={() => { if (tab.id === activeTab) changeTab(current => ({ ...current, activePane: node.id })); }}>
      <div className="flex h-10 shrink-0 items-center gap-1 overflow-x-auto border-b px-2">
        <Tabs.List aria-label="分屏工具" className="inline-flex shrink-0 items-center gap-1">
          {canTerminal && <Tabs.Trigger value="terminal" asChild><Button variant={node.kind === "terminal" ? "secondary" : "ghost"} size="sm" className="h-8 px-2"><SquareTerminal className="size-4" />终端</Button></Tabs.Trigger>}
          {canFiles && <Tabs.Trigger value="files" asChild><Button variant={node.kind === "files" ? "secondary" : "ghost"} size="sm" className="h-8 px-2"><FolderOpen className="size-4" />文件</Button></Tabs.Trigger>}
        </Tabs.List>
        <div className="min-w-0 flex-1" />
        <Button variant="ghost" size="icon" className="size-7" aria-label="左右分屏" title="左右分屏" onClick={() => split(node.id, "horizontal", node.kind)}><Columns2 className="size-4" /></Button>
        <Button variant="ghost" size="icon" className="size-7" aria-label="上下分屏" title="上下分屏" onClick={() => split(node.id, "vertical", node.kind)}><Rows2 className="size-4" /></Button>
        {canClose && <Button variant="ghost" size="icon" className="size-7" aria-label="关闭分屏" title="关闭分屏" onClick={() => closePane(node.id)}><X className="size-4" /></Button>}
      </div>
      <div className="min-h-0 min-w-0 flex-1 overflow-auto p-2">
        {node.opened.includes("terminal") && <Tabs.Content value="terminal" forceMount className={node.kind === "terminal" ? "h-full min-h-0" : "hidden"}><Suspense fallback={<p role="status">正在加载终端…</p>}><HostTerminal workspace={workspace} hostId={hostId} {...(node.directory ? { directory: node.directory } : {})} /></Suspense></Tabs.Content>}
        {node.opened.includes("files") && <Tabs.Content value="files" forceMount className={node.kind === "files" ? "h-full min-h-0" : "hidden"}><HostFiles workspace={workspace} hostId={hostId} canTerminal={canTerminal} {...(node.directory ? { initialPath: node.directory } : {})} onOpenTerminal={directory => split(node.id, "horizontal", "terminal", directory)} /></Tabs.Content>}
      </div>
    </section></Tabs.Root>;
  };

  if (!canTerminal && !canFiles) return <p role="alert" className="p-6">当前角色没有终端或文件权限。</p>;
  return <HostTelemetry workspace={workspace} hostId={hostId} enabled={tabs.some(tab => tab.id === activeTab && hasTerminal(tab.root))}><div className="flex h-dvh min-h-[420px] min-w-0 flex-col bg-background text-foreground">
    <header className="flex h-12 shrink-0 items-center gap-3 border-b px-3">
      <a href={href({ workspace, group: "host", item: "hosts", id: hostId, mode: "view" }, {})} className="inline-flex size-8 items-center justify-center rounded hover:bg-accent" aria-label="返回主机详情" title="返回主机详情"><ArrowLeft className="size-4" /></a>
      <h1 className="min-w-0 flex-1 truncate text-sm font-semibold" title={hostName}>{hostName}</h1>
      <HostLatency workspace={workspace} hostId={hostId} />
      {canTerminal && <Button variant="outline" size="icon" aria-label="新建终端" title="新建终端" onClick={() => addTab("terminal")}><SquareTerminal className="size-4" /></Button>}
      {canFiles && <Button variant="outline" size="icon" aria-label="新建文件标签" title="新建文件标签" onClick={() => addTab("files")}><FolderOpen className="size-4" /></Button>}
    </header>
    {error && <p role="alert" className="border-b px-3 py-1 text-sm text-destructive">{error}</p>}
    <div role="tablist" aria-label="主机工作标签" className="flex h-10 shrink-0 items-center gap-1 overflow-x-auto border-b px-2">
      {tabs.map(tab => <div key={tab.id} className={`inline-flex h-8 shrink-0 items-center rounded border ${activeTab === tab.id ? "bg-accent" : ""}`}>
        <button type="button" role="tab" aria-selected={activeTab === tab.id} className="h-full px-3 text-sm" onClick={() => setActiveTab(tab.id)}>{tab.label}</button>
        <Button variant="ghost" size="icon" className="size-7" aria-label={`关闭${tab.label}`} onClick={() => closeTab(tab.id)}><X className="size-3.5" /></Button>
      </div>)}
      <DropdownMenu><DropdownMenuTrigger asChild><Button variant="ghost" size="icon" className="size-7 shrink-0" aria-label="新建标签" title="新建标签"><Plus className="size-4" /></Button></DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          {canTerminal && <DropdownMenuItem onSelect={() => addTab("terminal")}><SquareTerminal className="size-4" />新建终端</DropdownMenuItem>}
          {canFiles && <DropdownMenuItem onSelect={() => addTab("files")}><FolderOpen className="size-4" />新建文件管理</DropdownMenuItem>}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
    {tabs.map(tab => <div key={tab.id} role="tabpanel" className={activeTab === tab.id ? "relative min-h-0 min-w-0 flex-1 overflow-hidden max-[480px]:flex max-[480px]:flex-col max-[480px]:overflow-y-auto" : "hidden"}>
      {/* Flat keyed siblings preserve live terminal sessions when the split tree changes. */}
      {arrange(tab.root).map(({ node, rect }) => <div key={node.id} className="absolute left-[var(--pane-left)] top-[var(--pane-top)] flex h-[var(--pane-height)] w-[var(--pane-width)] min-h-0 min-w-0 overflow-hidden border-r border-b max-[480px]:relative max-[480px]:top-0 max-[480px]:left-0 max-[480px]:h-[420px] max-[480px]:w-full max-[480px]:shrink-0"
        style={{ "--pane-left": `${rect.left}%`, "--pane-top": `${rect.top}%`, "--pane-width": `${rect.width}%`, "--pane-height": `${rect.height}%` } as CSSProperties}>
        {renderPane(node, tab, "children" in tab.root)}
      </div>)}
    </div>)}
  </div></HostTelemetry>;
}
