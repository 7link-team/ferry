import { useTerminalPalette } from "./terminal-palette";
import { ArrowLeft,Columns2,FolderOpen,Plus,Rows2,SquareTerminal,X } from "lucide-react";
import { Tabs } from "radix-ui";
import { lazy,Suspense,useEffect,useState,type CSSProperties,type ReactNode } from "react";
import { api,errorText } from "../../shared/api/api";
import { commonText,hostRepositoryText,useCommonTranslation } from "../../shared/i18n/i18n";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { DropdownMenu,DropdownMenuContent,DropdownMenuItem,DropdownMenuTrigger } from "../../shared/ui/dropdown-menu";
import { HostFiles } from "./HostFiles";
import { HostLatency } from "./HostLatency";
import { HostTelemetry } from "./HostTelemetry";

const HostTerminal = lazy(() => import("./HostTerminal").then(module => ({ default: module.HostTerminal })));

type Kind = "terminal" | "files";
type Pane = { id: string; kind: Kind; opened: Kind[]; directory?: string };
type Node = Pane | { id: string; axis: "horizontal" | "vertical"; children: [Node, Node] };
type WorkspaceTab = { id: string; root: Node; activePane: string; labelKind: Kind; number: number };
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

export function HostWorkspace({ navigation, workspace, hostId, permissions, headerActions }: { navigation: Navigation; workspace: string; hostId: string; permissions: Set<string>; headerActions?: ReactNode }) {
  useCommonTranslation();
  const { palette: terminalPalette, setPalette: setTerminalPalette } = useTerminalPalette();
  const canTerminal = permissions.has("hosts.terminal");
  const canFiles = permissions.has("hosts.files");
  const initialKind: Kind = new URLSearchParams(location.search).get("tool") === "files" && canFiles ? "files" : canTerminal ? "terminal" : "files";
  const initialDirectory = new URLSearchParams(location.search).get("directory") ?? undefined;
  const [tabs, setTabs] = useState<WorkspaceTab[]>(() => {
    const root = pane(initialKind, initialDirectory);
    return [{ id: id(), root, activePane: root.id, labelKind: initialKind, number: 1 }];
  });
  const [activeTab, setActiveTab] = useState(tabs[0]!.id);
  const [hostName, setHostName] = useState("");
  const tabLabel = (tab: WorkspaceTab) => `${hostRepositoryText(tab.labelKind === "terminal" ? "host.071" : "host.072")} ${tab.number}`;
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
    const tab = { id: id(), root, activePane: root.id, labelKind: kind, number: tabs.filter(item => firstPane(item.root).kind === kind).length + 1 };
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
      const replacement = { id: id(), root, activePane: root.id, labelKind: (canTerminal ? "terminal" : "files") as Kind, number: 1 };
      setTabs([replacement]); setActiveTab(replacement.id); return;
    }
    setTabs(remaining);
    if (activeTab === target) setActiveTab(remaining.at(-1)!.id);
  };

  const renderPane = (node: Pane, tab: WorkspaceTab, canClose: boolean): React.ReactNode => {
    return <Tabs.Root value={node.kind} onValueChange={value => changeTab(current => ({ ...current, root: replace(current.root, node.id, old => ({ ...old, kind: value as Kind, opened: [...new Set([...old.opened, value as Kind])] })) }))} asChild><section aria-label={node.kind === "terminal" ? hostRepositoryText("host.073") : hostRepositoryText("host.074")} className={`flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden ${tab.activePane === node.id ? "bg-accent/20" : ""}`} onPointerDown={() => { if (tab.id === activeTab) changeTab(current => ({ ...current, activePane: node.id })); }}>
      <div className="flex h-10 shrink-0 items-center gap-1 overflow-x-auto border-b px-2">
        <Tabs.List aria-label={hostRepositoryText("host.075")} className="inline-flex shrink-0 items-center gap-1">
          {canTerminal && <Tabs.Trigger value="terminal" asChild><Button variant={node.kind === "terminal" ? "secondary" : "ghost"} size="sm" className="h-8 px-2"><SquareTerminal className="size-4" />{hostRepositoryText("host.071")}</Button></Tabs.Trigger>}
          {canFiles && <Tabs.Trigger value="files" asChild><Button variant={node.kind === "files" ? "secondary" : "ghost"} size="sm" className="h-8 px-2"><FolderOpen className="size-4" />{hostRepositoryText("host.072")}</Button></Tabs.Trigger>}
        </Tabs.List>
        <div className="min-w-0 flex-1" />
        <Button variant="ghost" size="icon" className="size-7" aria-label={hostRepositoryText("host.076")} title={hostRepositoryText("host.076")} onClick={() => split(node.id, "horizontal", node.kind)}><Columns2 className="size-4" /></Button>
        <Button variant="ghost" size="icon" className="size-7" aria-label={hostRepositoryText("host.077")} title={hostRepositoryText("host.077")} onClick={() => split(node.id, "vertical", node.kind)}><Rows2 className="size-4" /></Button>
        {canClose && <Button variant="ghost" size="icon" className="size-7" aria-label={hostRepositoryText("host.078")} title={hostRepositoryText("host.078")} onClick={() => closePane(node.id)}><X className="size-4" /></Button>}
      </div>
      <div className="min-h-0 min-w-0 flex-1 overflow-auto p-2">
        {node.opened.includes("terminal") && <Tabs.Content value="terminal" forceMount className={node.kind === "terminal" ? "h-full min-h-0" : "hidden"}><Suspense fallback={<p role="status">{hostRepositoryText("host.079")}</p>}><HostTerminal workspace={workspace} hostId={hostId} {...(node.directory ? { directory: node.directory } : {})} /></Suspense></Tabs.Content>}
        {node.opened.includes("files") && <Tabs.Content value="files" forceMount className={node.kind === "files" ? "h-full min-h-0" : "hidden"}><HostFiles active={tab.id === activeTab && node.id === tab.activePane && node.kind === "files"} workspace={workspace} hostId={hostId} canTerminal={canTerminal} initialPath={node.directory ?? "/"} onOpenTerminal={directory => split(node.id, "horizontal", "terminal", directory)} /></Tabs.Content>}
      </div>
    </section></Tabs.Root>;
  };

  if (!canTerminal && !canFiles) return <p role="alert" className="p-6">{hostRepositoryText("host.080")}</p>;
  return <HostTelemetry workspace={workspace} hostId={hostId} enabled={tabs.some(tab => tab.id === activeTab && hasTerminal(tab.root))}><div className="flex h-dvh min-h-[420px] min-w-0 flex-col bg-background text-foreground">
    <header className="flex min-h-12 shrink-0 flex-wrap items-center gap-3 border-b px-3">
      <a href={navigation.href({ workspace, group: "host", item: "hosts", id: hostId, mode: "view" }, {})} className="inline-flex size-8 items-center justify-center rounded hover:bg-accent" aria-label={hostRepositoryText("host.081")} title={hostRepositoryText("host.081")}><ArrowLeft className="size-4" /></a>
      <h1 className="min-w-0 flex-1 truncate text-sm font-semibold" title={hostName || hostRepositoryText("host.070")}>{hostName || hostRepositoryText("host.070")}</h1>
      <HostLatency workspace={workspace} hostId={hostId} />
      {canTerminal && <Button variant="outline" size="icon" aria-label={hostRepositoryText("host.082")} title={hostRepositoryText("host.082")} onClick={() => addTab("terminal")}><SquareTerminal className="size-4" /></Button>}
      {canFiles && <Button variant="outline" size="icon" aria-label={hostRepositoryText("host.083")} title={hostRepositoryText("host.083")} onClick={() => addTab("files")}><FolderOpen className="size-4" /></Button>}
      <select aria-label={commonText("terminal.palette")} className="h-8 min-w-0 max-w-40 rounded-md border border-input bg-background px-2 text-sm font-medium" value={terminalPalette} onChange={event => setTerminalPalette(event.target.value)}>{["theme", "classic", "light", "dracula", "solarized"].map(value => <option key={value} value={value}>{commonText(`terminal.${value}`)}</option>)}</select>
      {headerActions}
    </header>
    {error && <p role="alert" className="border-b px-3 py-1 text-sm text-destructive">{error}</p>}
    <div role="tablist" aria-label={hostRepositoryText("host.084")} className="flex h-10 shrink-0 items-center gap-1 overflow-x-auto border-b px-2">
      {tabs.map(tab => <div key={tab.id} className={`inline-flex h-8 shrink-0 items-center rounded border ${activeTab === tab.id ? "bg-accent" : ""}`}>
        <button type="button" role="tab" aria-selected={activeTab === tab.id} className="h-full px-3 text-sm" onClick={() => setActiveTab(tab.id)}>{tabLabel(tab)}</button>
        <Button variant="ghost" size="icon" className="size-7" aria-label={hostRepositoryText("host.085", {value1: tabLabel(tab)})} onClick={() => closeTab(tab.id)}><X className="size-3.5" /></Button>
      </div>)}
      <DropdownMenu><DropdownMenuTrigger asChild><Button variant="ghost" size="icon" className="size-7 shrink-0" aria-label={hostRepositoryText("host.086")} title={hostRepositoryText("host.086")}><Plus className="size-4" /></Button></DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          {canTerminal && <DropdownMenuItem onSelect={() => addTab("terminal")}><SquareTerminal className="size-4" />{hostRepositoryText("host.082")}</DropdownMenuItem>}
          {canFiles && <DropdownMenuItem onSelect={() => addTab("files")}><FolderOpen className="size-4" />{hostRepositoryText("host.087")}</DropdownMenuItem>}
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
