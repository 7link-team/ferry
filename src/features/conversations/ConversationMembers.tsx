import { useState, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { ArrowLeft, MessageSquare, Search, X } from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { base } from "../../shared/api/resource";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { IdentityAvatar, type AvatarValue } from "../../shared/ui/identity-avatar";
import { conversationText as t } from "./resource-config";

export interface ConversationMember {
  id: string; name: string; kind: "user" | "agent";
  avatar?: AvatarValue | null | undefined;
  runtime?: string | undefined; model?: string | undefined; revision_id?: string | undefined;
}
interface MemberTurn {
  id: string; agent_id?: string; state: string;
  decisions: Record<string, { decision: boolean | null; sent: boolean }>;
}
export function memberWorkState(turns: MemberTurn[], connected: boolean) {
  if (!connected) return "statusUnknown";
  const current = turns.find(turn => ["queued", "running", "stopping"].includes(turn.state));
  if (!current) return "statusIdle";
  if (current.state === "stopping") return "stopping";
  if (Object.values(current.decisions).some(decision => decision.decision === null && !decision.sent)) return "statusApproval";
  return current.state;
}
export function ConversationMembers({ open, onOpenChange, selected, onSelect, title, members, turns, connected,
  ownerId, viewerId, primaryAgentId, workspace, permissions, onOpenConversation, manage, executionDetails }: {
  open: boolean; onOpenChange: (value: boolean) => void;
  selected: string; onSelect: (value: string) => void; title: string;
  members: ConversationMember[]; turns: MemberTurn[]; connected: boolean;
  ownerId: string; viewerId: string; primaryAgentId: string; workspace: string;
  permissions: Set<string>; onOpenConversation?: ((id: string) => void) | undefined;
  manage: ReactNode; executionDetails: ReactNode;
}) {
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const member = members.find(item => `${item.kind}:${item.id}` === selected);
  const memberTurns = (item: ConversationMember) => item.kind === "agent" ? turns.filter(turn => (turn.agent_id || primaryAgentId) === item.id) : [];
  const status = (item: ConversationMember) => memberWorkState(memberTurns(item), connected);
  const filtered = members.filter(item => `${item.name} ${item.id}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase()));
  const canChat = member?.kind === "agent" && onOpenConversation && ["conversations.create", "ai-agents.read", "ai-agents.use"].every(permission => permissions.has(permission));
  async function startChat() {
    if (!member || !canChat) return;
    setBusy(true); setError("");
    try {
      const row = await api<{ id: string }>(base(workspace, "conversations"), { method: "POST", body: JSON.stringify({ agent_id: member.id, title: member.name }) });
      onOpenChange(false); onOpenConversation?.(row.id);
    } catch (cause) { setError(errorText(cause)); }
    finally { setBusy(false); }
  }
  return <Dialog.Root open={open} onOpenChange={value => { if (!busy) { onOpenChange(value); setError(""); } }}>
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
      <Dialog.Content aria-describedby={undefined} className="fixed inset-y-0 right-0 z-50 flex w-full max-w-lg flex-col border-l bg-background text-foreground shadow-xl">
        <header className="flex shrink-0 items-center gap-2 border-b px-4 py-3">
          {member && <Button variant="ghost" size="icon" className="size-11" aria-label={t("conversationMembers")} onClick={() => { onSelect(""); setError(""); }}><ArrowLeft /></Button>}
          <div className="min-w-0 flex-1"><Dialog.Title className="font-semibold">{t(member ? "memberDetails" : "conversationMembers")}</Dialog.Title><p className="truncate text-xs text-muted-foreground">{title}</p></div>
          <Dialog.Close asChild><Button variant="ghost" size="icon" className="size-11" disabled={busy} aria-label={t("close")}><X /></Button></Dialog.Close>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-4">
          {member ? <>
            <div className="flex flex-col items-center gap-3 py-5 text-center"><IdentityAvatar {...member} value={member.avatar} className="size-24 text-3xl" /><h3 className="max-w-full break-words text-xl font-semibold">{member.name}</h3><p className="text-sm text-muted-foreground">{t(member.kind === "agent" ? "agent" : "member")}{member.id === viewerId && member.kind === "user" ? ` · ${t("you")}` : ""}</p></div>
            <dl className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-x-4 gap-y-4 rounded-xl border p-4 text-sm">
              <dt className="text-muted-foreground">ID</dt><dd className="break-all font-mono text-xs">{member.id}</dd>
              {member.kind === "agent" && <><dt className="text-muted-foreground">{t("memberWorkStatus")}</dt><dd>{t(status(member))}</dd><dt className="text-muted-foreground">{t("lastResult")}</dt><dd>{memberTurns(member).filter(turn => !["queued", "running", "stopping"].includes(turn.state)).at(-1)?.state ? t(memberTurns(member).filter(turn => !["queued", "running", "stopping"].includes(turn.state)).at(-1)!.state) : t("noExecution")}</dd><dt className="text-muted-foreground">{t("runtime")}</dt><dd className="break-words">{member.runtime || "—"}</dd><dt className="text-muted-foreground">{t("model")}</dt><dd className="break-words">{member.model || t("nativeModel")}</dd>{member.revision_id && <><dt className="text-muted-foreground">{t("revision")}</dt><dd className="break-all">{member.revision_id}</dd></>}</>}
              {member.kind === "user" && <><dt className="text-muted-foreground">{t("memberRole")}</dt><dd>{t(member.id === ownerId ? "conversationOwner" : "member")}</dd></>}
            </dl>
            <p className="mt-3 text-xs leading-relaxed text-muted-foreground">{t(member.kind === "agent" ? "memberStatusScope" : "humanPresenceUnavailable")}</p>
          </> : <>
            <div className="relative mb-4"><Search className="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground" /><Input className="min-h-11 pl-9" value={search} onChange={event => setSearch(event.target.value)} placeholder={t("searchMembers")} aria-label={t("searchMembers")} /></div>
            <p className="mb-3 text-xs text-muted-foreground">{t("memberStatusScope")}</p>
            <ul className="space-y-1">{filtered.map(item => <li key={`${item.kind}:${item.id}`}><button type="button" className="flex min-h-20 w-full items-center gap-3 rounded-xl border border-transparent px-3 py-3 text-left hover:border-border hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring" onClick={() => onSelect(`${item.kind}:${item.id}`)}><IdentityAvatar {...item} value={item.avatar} className="size-12" /><span className="min-w-0 flex-1"><span className="block truncate font-medium">{item.name}</span><span className="block text-xs text-muted-foreground">{t(item.kind === "agent" ? "agent" : "member")}{item.id === viewerId && item.kind === "user" ? ` · ${t("you")}` : ""}</span></span>{item.kind === "agent" && <span className={`max-w-[45%] rounded-full px-2.5 py-1 text-xs ${["running", "queued", "statusApproval", "stopping"].includes(status(item)) ? "bg-primary/15 text-primary" : "bg-muted text-muted-foreground"}`}>{t(status(item))}</span>}</button></li>)}</ul>
            {!filtered.length && <p role="status" className="py-8 text-center text-sm text-muted-foreground">{t("noMatchingMembers")}</p>}
            {executionDetails}
          </>}
          {error && <p role="alert" className="mt-4 text-sm text-destructive">{error}</p>}
        </div>
        {(canChat || !member && manage) && <footer className="shrink-0 border-t bg-background p-4 pb-[max(1rem,env(safe-area-inset-bottom))] [&>button]:min-h-11 [&>button]:w-full">{member ? <Button disabled={busy} onClick={() => void startChat()}><MessageSquare />{t(busy ? "creatingConversation" : "startDirectChat")}</Button> : manage}</footer>}
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
