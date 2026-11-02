import { cn } from "cn";
import { ArrowDown,ArrowUp,FolderTree,Pencil,Plus,Trash2 } from "lucide-react";
import { Dialog } from "radix-ui";
import { useEffect,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { base,createResource,deleteResource,listResource,patchResource } from "../../shared/api/resource";
import { AutoField,AutoForm,FieldControl } from "../../shared/forms/auto-form";
import { commonText,useCommonTranslation } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import { Button } from "../../shared/ui/button";
import { DeleteConfirmDialog } from "../../shared/ui/DeleteConfirmDialog";
import { Input } from "../../shared/ui/input";

type GroupNode = { id: string; name: string; description: string; parent_id?: string; position?: number; version: number };
export function GroupSidebar({ workspace, permissions, resource, noun, active, onSelect }: {
  workspace: string; permissions: Set<string>; resource: "host-groups" | "pipeline-groups"; noun: string; active: string; onSelect: (id: string) => void;
}) {
  useCommonTranslation();
  const [groups, setGroups] = useState<GroupNode[]>([]);
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState("");
  const [editing, setEditing] = useState<{ group?: GroupNode; parentId: string }>();
  const [deleting, setDeleting] = useState<GroupNode>();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const canCreate = permissions.has(`${resource}.create`);
  useEffect(() => {
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
  }, [workspace, resource, revision]);
  const open = (group?: GroupNode, parentId = "") => { setEditing({ ...(group ? { group } : {}), parentId }); setName(group?.name ?? ""); setDescription(group?.description ?? ""); setError(""); };
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
    <div className={cn("group flex min-w-0 items-center rounded", active === group.id ? "bg-accent" : "hover:bg-accent")} style={{ paddingLeft: depth * 12 }}>
      <button type="button" aria-current={active === group.id ? "page" : undefined} className="min-w-0 flex-1 truncate px-2 py-1.5 text-left text-sm" title={group.name} onClick={() => onSelect(group.id)}>{group.name}</button>
      {permissions.has(`${resource}.update`) && <>
        <Button variant="ghost" size="icon" className="size-7 shrink-0 hover:bg-foreground/15 focus-visible:bg-foreground/15" aria-label={commonText("group.moveUp", { name: group.name })} title={commonText("group.up")} disabled={busy || siblings(group.parent_id ?? "")[0]?.id === group.id} onClick={() => void moveGroup(group, -1)}><ArrowUp className="size-3.5" /></Button>
        <Button variant="ghost" size="icon" className="size-7 shrink-0 hover:bg-foreground/15 focus-visible:bg-foreground/15" aria-label={commonText("group.moveDown", { name: group.name })} title={commonText("group.down")} disabled={busy || siblings(group.parent_id ?? "").at(-1)?.id === group.id} onClick={() => void moveGroup(group, 1)}><ArrowDown className="size-3.5" /></Button>
      </>}
      {resource === "host-groups" && canCreate && <Button variant="ghost" size="icon" className="size-7 shrink-0 hover:bg-foreground/15 focus-visible:bg-foreground/15" aria-label={commonText("group.createUnder", { name: group.name })} title={commonText("group.newChild")} onClick={() => open(undefined, group.id)}><Plus className="size-3.5" /></Button>}
      {permissions.has(`${resource}.update`) && <Button variant="ghost" size="icon" className="size-7 shrink-0 hover:bg-foreground/15 focus-visible:bg-foreground/15" aria-label={commonText("group.editNamed", { name: group.name })} title={commonText("group.edit")} onClick={() => open(group, group.parent_id ?? "")}><Pencil className="size-3.5" /></Button>}
      {permissions.has(`${resource}.delete`) && <Button variant="ghost" size="icon" className="size-7 shrink-0 hover:bg-foreground/15 focus-visible:bg-foreground/15" aria-label={commonText("group.deleteNamed", { name: group.name })} title={commonText("group.deleteNamed", { name: group.name })} onClick={() => setDeleting(group)}><Trash2 className="size-3.5" /></Button>}
    </div>
    {resource === "host-groups" && siblings(group.id).map(child => renderGroup(child, depth + 1))}
  </div>;
  return <aside aria-label={resource === "host-groups" ? commonText("group.hostGroups") : commonText("group.resourceGroups", { name: noun })} className="flex w-full shrink-0 flex-col gap-1 border-b pb-3 sm:w-56 sm:overflow-y-auto sm:border-r sm:border-b-0 sm:pr-3">
    {deleting && <DeleteConfirmDialog action={commonText("group.delete")} target={deleting.name} onClose={() => setDeleting(undefined)} onConfirm={async () => {
      await deleteResource(workspace, resource, deleting.id);
      if (active === deleting.id) onSelect("");
      setRevision(value => value + 1);
    }} />}
    <div className="mb-1 flex items-center gap-1 px-2 text-sm font-medium"><FolderTree className="size-4" />{commonText("table.groups")}<span className="flex-1" />
      {canCreate && <Button variant="ghost" size="icon" className="size-7 hover:bg-foreground/15 focus-visible:bg-foreground/15" aria-label={commonText("group.create")} title={commonText("group.create")} onClick={() => open()}><Plus className="size-4" /></Button>}
    </div>
    <button type="button" aria-current={active === "" ? "page" : undefined} className={cn("rounded px-2 py-1.5 text-left text-sm", active === "" ? "bg-accent font-medium" : "hover:bg-accent")} onClick={() => onSelect("")}>{commonText("group.all")}{noun}</button>
    <button type="button" aria-current={active === "0" ? "page" : undefined} className={cn("rounded px-2 py-1.5 text-left text-sm", active === "0" ? "bg-accent font-medium" : "hover:bg-accent")} onClick={() => onSelect("0")}>{commonText("group.ungrouped")}{noun}</button>
    {(resource === "host-groups" ? groups.filter(group => !group.parent_id || group.parent_id === "0" || !groups.some(parent => parent.id === group.parent_id)) : groups).sort((a, b) => (a.position ?? 0) - (b.position ?? 0) || a.name.localeCompare(b.name, currentLocale())).map(group => renderGroup(group, 0))}
    {error && !editing && <p role="alert" className="px-2 text-xs text-destructive">{error}</p>}
    <Dialog.Root open={!!editing} onOpenChange={value => { if (!value && !busy) setEditing(undefined); }}><Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" /><Dialog.Content className="fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border bg-background p-6 shadow-lg">
      <Dialog.Title className="text-lg font-semibold">{editing?.group ? commonText("group.edit") : commonText("group.create")}</Dialog.Title>
      <Dialog.Description className="sr-only">{commonText("group.help")}</Dialog.Description>
      <AutoForm className="mt-4 space-y-4" onSubmit={async event => {
        event.preventDefault(); if (!editing || !name.trim()) return;
        setBusy(true); setError("");
        try {
          const values = { name: name.trim(), description, ...(resource === "host-groups" ? { parent_id: editing.parentId } : {}) };
          if (editing.group) await patchResource(workspace, resource, editing.group.id, { ...values, version: editing.group.version });
          else await createResource(workspace, resource, values);
          setEditing(undefined); setRevision(value => value + 1);
        } catch (cause) { setError(errorText(cause)); }
        finally { setBusy(false); }
      }}>
        <AutoField label={commonText("common.name")} className="block space-y-2 text-sm"><Input value={name} maxLength={60} required onChange={event => setName(event.target.value)} /></AutoField>
        {resource === "host-groups" && <AutoField label={commonText("repository.parent")} className="block space-y-2 text-sm"><FieldControl name="parent_id" field={{ name: commonText("repository.parent"), type: "select", config: { emptyValues: [0, "0"], emptyLabel: commonText("group.topLevel"), options: groups.filter(group => group.id !== editing?.group?.id).map(group => ({ value: group.id, label: group.name })) } }} value={editing?.parentId ?? ""} onChange={value => setEditing(current => current ? { ...current, parentId: String(value) } : current)} /></AutoField>}
        <AutoField label={commonText("common.description")} className="block space-y-2 text-sm"><Input value={description} onChange={event => setDescription(event.target.value)} /></AutoField>
        {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
        <div className="flex justify-end gap-2"><Button type="button" variant="outline" onClick={() => setEditing(undefined)}>{commonText("common.cancel")}</Button><Button type="submit" disabled={busy || !name.trim()}>{busy ? commonText("common.saving") : commonText("common.save")}</Button></div>
      </AutoForm>
    </Dialog.Content></Dialog.Portal></Dialog.Root>
  </aside>;
}
