import { navigate } from "../router";
import { cn } from "cn";
import { ChevronDown,ChevronUp,EllipsisVertical,FolderTree,Pencil,Plus,Trash2 } from "lucide-react";
import { useEffect,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { base,deleteResource,listResource } from "../../shared/api/resource";
import { commonText,useCommonTranslation } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import { Button } from "../../shared/ui/button";
import { DeleteConfirmDialog } from "../../shared/ui/DeleteConfirmDialog";
import { DropdownMenu,DropdownMenuContent,DropdownMenuItem,DropdownMenuTrigger } from "../../shared/ui/dropdown-menu";

export type GroupNode = { id: string; name: string; description: string; parent_id?: string; position?: number; version: number; count?: number };
export function GroupSidebar({ workspace, permissions, resource, noun, active, onSelect, groups: suppliedGroups, total }: {
  workspace: string; permissions: Set<string>; resource: "host-groups" | "pipeline-groups" | "settings" | "dictionaries"; noun: string; active: string; onSelect: (id: string) => void; groups?: GroupNode[]; total?: number;
}) {
  useCommonTranslation();
  const [groups, setGroups] = useState<GroupNode[]>([]);
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState("");
  const [deleting, setDeleting] = useState<GroupNode>();
  const [busy, setBusy] = useState(false);
  const canCreate = suppliedGroups === undefined && permissions.has(`${resource}.create`);
  const canUpdate = suppliedGroups === undefined && permissions.has(`${resource}.update`);
  const canDelete = suppliedGroups === undefined && permissions.has(`${resource}.delete`);
  useEffect(() => {
    if (suppliedGroups !== undefined) { setGroups(suppliedGroups); return; }
    let live = true;
    const load = async () => {
      const rows: GroupNode[] = [];
      for (let page = 1; ; page++) {
        const result = await listResource<GroupNode>(workspace, resource, { page, page_size: 200 });
        rows.push(...result.items);
        if (page * result.page_size >= result.total) break;
      }
      if (live) { setGroups(rows); setError(""); }
    };
    void load().catch(cause => { if (live) setError(errorText(cause)); });
    return () => { live = false; };
  }, [workspace, resource, revision, suppliedGroups]);
  const open = (group?: GroupNode, parentId = "") => navigate(
    { workspace, group: resource === "host-groups" ? "host" : "workflow", item: resource, mode: group ? "edit" : "new", ...(group ? { id: group.id } : {}) },
    { return_to: location.pathname + location.search + location.hash, ...(parentId && !group ? { parent_id: parentId } : {}) },
  );
  const siblings = (parentId: string) => groups.filter(group => (group.parent_id || "0") === (parentId || "0"))
    .sort((a, b) => (a.position ?? 0) - (b.position ?? 0) || a.name.localeCompare(b.name, currentLocale()) || a.id.localeCompare(b.id));
  const moveGroup = async (group: GroupNode, direction: -1 | 1) => {
    const rows = siblings(group.parent_id ?? "");
    const index = rows.findIndex(row => row.id === group.id);
    const next = index + direction;
    if (next < 0 || next >= rows.length || busy) return;
    [rows[index], rows[next]] = [rows[next]!, rows[index]!];
    setBusy(true); setError("");
    try {
      await api(`${base(workspace, resource)}/ordering`, {
        method: "PATCH", body: JSON.stringify(resource === "host-groups" ? { parent_id: group.parent_id ?? "", ids: rows.map(row => row.id) } : { ids: rows.map(row => row.id) }),
      });
      setRevision(value => value + 1);
    } catch (cause) { setError(errorText(cause)); }
    finally { setBusy(false); }
  };
  const renderGroup = (group: GroupNode, depth: number): React.ReactNode => <div key={group.id}>
    <div className={cn("group flex min-w-0 items-center rounded", active === group.id ? "bg-primary font-medium text-primary-foreground" : "hover:bg-accent hover:text-accent-foreground")} style={{ paddingLeft: depth * 12 }}>
      <button type="button" aria-current={active === group.id ? "page" : undefined} className="flex min-w-0 flex-1 items-center justify-between gap-2 px-2 py-1.5 text-left text-sm" title={group.name} onClick={() => onSelect(group.id)}><span className="truncate">{group.name}</span>{group.count !== undefined && <span className="shrink-0 tabular-nums opacity-70">{group.count}</span>}</button>
      {(canUpdate || canDelete || (resource === "host-groups" && canCreate)) && <DropdownMenu>
        <DropdownMenuTrigger asChild><Button variant="ghost" size="icon" className="size-7 text-inherit hover:bg-foreground/15 hover:text-inherit focus-visible:bg-foreground/15" aria-label={`${commonText("table.actions")} · ${group.name}`} title={commonText("table.actions")}><EllipsisVertical className="size-4" /></Button></DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {canUpdate && <DropdownMenuItem disabled={busy || siblings(group.parent_id ?? "")[0]?.id === group.id} onSelect={() => void moveGroup(group, -1)}><ChevronUp />{commonText("group.up")}</DropdownMenuItem>}
          {canUpdate && <DropdownMenuItem disabled={busy || siblings(group.parent_id ?? "").at(-1)?.id === group.id} onSelect={() => void moveGroup(group, 1)}><ChevronDown />{commonText("group.down")}</DropdownMenuItem>}
          {resource === "host-groups" && canCreate && <DropdownMenuItem disabled={busy} onSelect={() => open(undefined, group.id)}><Plus />{commonText("group.newChild")}</DropdownMenuItem>}
          {canUpdate && <DropdownMenuItem disabled={busy} onSelect={() => open(group, group.parent_id ?? "")}><Pencil />{commonText("group.edit")}</DropdownMenuItem>}
          {canDelete && <DropdownMenuItem variant="destructive" disabled={busy} onSelect={() => setDeleting(group)}><Trash2 />{commonText("group.delete")}</DropdownMenuItem>}
        </DropdownMenuContent>
      </DropdownMenu>}
    </div>
    {resource === "host-groups" && siblings(group.id).map(child => renderGroup(child, depth + 1))}
  </div>;
  return <aside aria-label={resource === "host-groups" ? commonText("group.hostGroups") : commonText("group.resourceGroups", { name: noun })} className="flex max-h-32 w-full shrink-0 flex-col gap-1 overflow-y-auto border-b pb-3 sm:max-h-none sm:w-56 sm:border-r sm:border-b-0 sm:pr-3">
    {deleting && <DeleteConfirmDialog action={commonText("group.delete")} target={deleting.name} onClose={() => setDeleting(undefined)} onConfirm={async () => {
      await deleteResource(workspace, resource, deleting.id);
      if (active === deleting.id) onSelect("");
      setRevision(value => value + 1);
    }} />}
    <div className="mb-1 flex items-center gap-1 rounded-md bg-muted/70 px-2 text-sm font-medium text-foreground"><FolderTree className="size-4" />{commonText("table.groups")}<span className="flex-1" />
      {canCreate && <Button variant="ghost" size="icon" className="size-7 hover:bg-foreground/15 focus-visible:bg-foreground/15" aria-label={commonText("group.create")} title={commonText("group.create")} onClick={() => open()}><Plus className="size-4" /></Button>}
    </div>
    <button type="button" aria-current={active === "" ? "page" : undefined} className={cn("flex items-center justify-between gap-2 rounded px-2 py-1.5 text-left text-sm", active === "" ? "bg-primary font-medium text-primary-foreground" : "hover:bg-accent hover:text-accent-foreground")} onClick={() => onSelect("")}>{commonText("group.all")}{total !== undefined && <span className="tabular-nums opacity-70">{total}</span>}</button>
    {suppliedGroups === undefined && <button type="button" aria-current={active === "0" ? "page" : undefined} className={cn("rounded px-2 py-1.5 text-left text-sm", active === "0" ? "bg-primary font-medium text-primary-foreground" : "hover:bg-accent hover:text-accent-foreground")} onClick={() => onSelect("0")}>{commonText("group.ungrouped")}</button>}
    {(resource === "host-groups" ? groups.filter(group => !group.parent_id || group.parent_id === "0" || !groups.some(parent => parent.id === group.parent_id)) : groups).sort((a, b) => (a.position ?? 0) - (b.position ?? 0) || a.name.localeCompare(b.name, currentLocale())).map(group => renderGroup(group, 0))}
    {error && <p role="alert" className="px-2 text-xs text-destructive">{error}</p>}

  </aside>;
}
