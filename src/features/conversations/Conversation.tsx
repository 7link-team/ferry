import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { ApiError, api, errorText } from "../../shared/api/api";
import { base } from "../../shared/api/resource";
import { AlertDialog, Dialog } from "radix-ui";
import {
  Trash2,
  Bot,
  User,
  Users,
  Info,
  Search,
  X,
  Copy,
  Check,
  ArrowDown,
  Bookmark,
  MessageSquare,
  Pencil,
  Undo2,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { AttachmentView, type Attachment } from "./AttachmentView";
import { Composer, type ComposerDraft } from "./Composer";
import { i18n, useCommonTranslation } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { conversationText as t } from "./resource-config";
export type ConversationDraft = ComposerDraft;
interface Decision {
  tool: string;
  input: unknown;
  decision: boolean | null;
  dispatching: boolean;
  sent: boolean;
}
interface Turn {
  id: string;
  owner_id: string;
  message_id?: string;
  agent_id?: string;
  state: string;
  prompt: string;
  attachments: Attachment[];
  error: string;
  usage: { input_tokens: number; output_tokens: number } | null;
  decisions: Record<string, Decision>;
}
interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  turn_id: string | null;
  reply_to: string | null;
  agent_id: string;
  sender_name: string;
  owner_id: string;
  prompt: string;
  attachments: Attachment[];
  targets: string[];
  created_at: number;
  version?: number;
  edited_at?: number | null;
  retracted_at?: number | null;
  mentions?: string[];
  saved?: boolean;
  unread?: boolean;
}
interface ConversationData {
  kind: "private" | "group";
  viewer_id: string;
  can_manage: boolean;
  members: { id: string; name: string }[];
  agents: { id: string; name: string; state: string }[];
  messages: ChatMessage[];
  id: string;
  title: string;
  state: string;
  revision_id: string;
  config: { name: string; runtime: string; model: string; cwd: string };
  binding: { execution_user: string; canonical_directory: string };
  turns: Turn[];
  version: number;
  owner_id: string;
  members_version: string;
}
interface Event {
  sequence: number;
  turn_id: string;
  event: {
    type: string;
    item_id?: string;
    text?: string;
    replace?: boolean;
    name?: string;
    state?: string;
  };
}
const active = (value: string) =>
  ["queued", "running", "stopping"].includes(value);
// Keep a room's human messages and concurrent Agent replies in actual arrival
// order. A reply link is metadata, not permission to move a later reply backwards.
export function conversationTimeline(messages: ChatMessage[], turns: Turn[]) {
  const entries: {
    id: string;
    created_at: number;
    message: ChatMessage | undefined;
    turn: Turn | undefined;
  }[] = messages.map((message) => ({
    id: message.id,
    created_at: message.created_at,
    message,
    turn: turns.find((turn) => turn.id === message.turn_id),
  }));
  for (const turn of turns) {
    if (
      !messages.some(
        (message) =>
          message.role === "assistant" && message.turn_id === turn.id,
      )
    ) {
      entries.push({
        id: turn.id,
        created_at:
          messages.find(
            (message) =>
              message.id === turn.message_id || message.turn_id === turn.id,
          )?.created_at ?? 0,
        message: undefined,
        turn,
      });
    }
  }
  return entries.sort(
    (a, b) => a.id.length - b.id.length || a.id.localeCompare(b.id),
  );
}
function CopyMessage({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return (
    <Button
      type="button"
      size="icon"
      variant="ghost"
      className="size-7"
      aria-label={t(copied ? "copiedMessage" : "copyMessage")}
      onClick={() => {
        void navigator.clipboard
          .writeText(text)
          .then(() => {
            setCopied(true);
            clearTimeout(timer.current);
            timer.current = setTimeout(() => setCopied(false), 1800);
          })
          .catch(() => {});
      }}
    >
      {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
    </Button>
  );
}
export function Conversation({
  workspace,
  id,
  permissions,
  headerActions,
  initialDraft,
  onDraftChange,
  onDeleted,
}: {
  workspace: string;
  id: string;
  permissions: Set<string>;
  headerActions?: ReactNode;
  initialDraft?: ConversationDraft | undefined;
  onDraftChange?: (draft: ConversationDraft) => void;
  onDeleted?: () => void;
}) {
  useCommonTranslation();
  const endpoint = `${base(workspace, "conversations")}/${encodeURIComponent(id)}`;
  const [data, setData] = useState<ConversationData>();
  const [events, setEvents] = useState<Event[]>([]);
  const [draft, setDraft] = useState(initialDraft?.prompt ?? "");
  const [attachments, setAttachments] = useState<Attachment[]>(
    initialDraft?.attachments ?? [],
  );
  const [targets, setTargets] = useState<string[]>(initialDraft?.targets ?? []);
  const [mentions, setMentions] = useState<string[]>(
    initialDraft?.mentions ?? [],
  );
  const [thread, setThread] = useState(initialDraft?.reply_to ?? "");
  const [activity, setActivity] = useState("allMessages");
  const [messageAction, setMessageAction] = useState<{
    message: ChatMessage;
    action: "edit" | "retract";
  }>();
  const [editDraft, setEditDraft] = useState("");
  const [actionError, setActionError] = useState("");
  const [memberPicker, setMemberPicker] = useState(false);
  const [memberOptions, setMemberOptions] = useState<
    { id: string; name: string }[]
  >([]);
  const [agentOptions, setAgentOptions] = useState<
    { id: string; name: string }[]
  >([]);
  const [memberIds, setMemberIds] = useState<string[]>([]);
  const [agentIds, setAgentIds] = useState<string[]>([]);
  const [membersVersion, setMembersVersion] = useState("");
  const [memberSearch, setMemberSearch] = useState("");
  useEffect(() => {
    onDraftChange?.({
      prompt: draft,
      attachments,
      targets,
      mentions,
      reply_to: thread,
    });
  }, [draft, attachments, targets, mentions, thread, onDraftChange]);
  const [deleting, setDeleting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [infoOpen, setInfoOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [newBelow, setNewBelow] = useState(false);
  const [error, setError] = useState("");
  const [connection, setConnection] = useState("");
  const [busy, setBusy] = useState(false);
  const pending = useRef<{ signature: string; client_id: string } | undefined>(
    undefined,
  );
  const cursor = useRef(0);
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const readCursor = useRef("");
  const lastMessage = data?.messages.at(-1)?.id;
  const executing = !!data?.turns.some((turn) => active(turn.state));
  useEffect(() => {
    if (!lastMessage || activity !== "allMessages" || thread || executing)
      return;
    const mark = () => {
      if (
        document.visibilityState !== "visible" ||
        !document.hasFocus() ||
        !follow.current ||
        readCursor.current === lastMessage
      )
        return;
      readCursor.current = lastMessage;
      void api(endpoint + "/read", {
        method: "POST",
        body: JSON.stringify({ message_id: lastMessage }),
      }).catch(() => {
        readCursor.current = "";
      });
    };
    mark();
    window.addEventListener("focus", mark);
    document.addEventListener("visibilitychange", mark);
    const node = scroll.current;
    node?.addEventListener("scroll", mark);
    return () => {
      window.removeEventListener("focus", mark);
      document.removeEventListener("visibilitychange", mark);
      node?.removeEventListener("scroll", mark);
    };
  }, [lastMessage, endpoint, activity, thread, executing]);
  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      try {
        const row = await api<ConversationData>(endpoint, {
          signal: controller.signal,
        });
        if (controller.signal.aborted) return;
        setData(row);
        let more = true;
        while (more && !controller.signal.aborted) {
          const result = await api<{
            items: Event[];
            next: number;
            has_more: boolean;
          }>(`${endpoint}/events?after=${cursor.current}`, {
            signal: controller.signal,
          });
          if (controller.signal.aborted) return;
          const previous = cursor.current;
          cursor.current = result.next;
          setEvents((old) => [
            ...old,
            ...result.items.filter((event) => event.sequence > previous),
          ]);
          more = result.has_more;
        }
        setConnection("");
      } catch (cause) {
        if (controller.signal.aborted) return;
        if (
          cause instanceof ApiError &&
          [401, 403, 404].includes(cause.status)
        ) {
          setData(undefined);
          setEvents([]);
          cursor.current = 0;
        }
        setConnection(errorText(cause));
      } finally {
        if (!controller.signal.aborted)
          timer = setTimeout(() => void refresh(), 1000);
      }
    }
    void refresh();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [endpoint]);
  const messageVersion = data?.messages
    .map((message) => `${message.id}:${message.prompt.length}`)
    .join(",");
  useEffect(() => {
    if (follow.current && scroll.current)
      scroll.current.scrollTop = scroll.current.scrollHeight;
    else if (data?.messages.length) setNewBelow(true);
  }, [messageVersion, data?.turns.length]);
  const loaded = !!data;
  useEffect(() => {
    const node = scroll.current;
    if (!node || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      if (follow.current) node.scrollTop = node.scrollHeight;
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, [loaded]);
  const current = data?.turns.find(
    (turn) => active(turn.state) && turn.owner_id === data.viewer_id,
  );
  const runnable =
    permissions.has("conversations.run") && data?.state === "active";
  async function act(path: string, body: unknown = {}) {
    setError("");
    setBusy(true);
    try {
      await api(endpoint + path, {
        method: "POST",
        body: JSON.stringify(body),
      });
      setData(await api<ConversationData>(endpoint));
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  async function send() {
    if (
      (!draft.trim() && !attachments.length) ||
      (current && data?.kind !== "group") ||
      !runnable ||
      busy
    )
      return;
    const signature = JSON.stringify({
      prompt: draft,
      attachments,
      targets,
      mentions,
      thread,
    });
    if (pending.current?.signature !== signature)
      pending.current = { signature, client_id: crypto.randomUUID() };
    setError("");
    setBusy(true);
    try {
      await api(endpoint + (data?.kind === "group" ? "/messages" : "/turns"), {
        method: "POST",
        body: JSON.stringify({
          prompt: draft,
          attachments,
          client_id: pending.current.client_id,
          ...(thread ? { reply_to: thread } : {}),
          ...(data?.kind === "group" ? { targets, mentions } : {}),
        }),
      });
      setDraft("");
      setAttachments([]);
      setTargets([]);
      setMentions([]);
      pending.current = undefined;
      follow.current = true;
      setData(await api<ConversationData>(endpoint));
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  async function removeConversation() {
    setDeleting(true);
    setError("");
    try {
      await api(endpoint, { method: "DELETE" });
      setConfirmDelete(false);
      onDeleted?.();
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setDeleting(false);
    }
  }
  async function mutateMessage(
    message: ChatMessage,
    method: "PATCH" | "DELETE" | "PUT",
    suffix = "",
    body?: unknown,
  ) {
    setActionError("");
    setError("");
    setBusy(true);
    try {
      await api(endpoint + "/messages/" + message.id + suffix, {
        method,
        ...(body ? { body: JSON.stringify(body) } : {}),
      });
      setData(await api<ConversationData>(endpoint));
      setMessageAction(undefined);
    } catch (cause) {
      const text = errorText(cause);
      setActionError(text);
      setError(text);
    } finally {
      setBusy(false);
    }
  }
  async function openMembers() {
    if (!data) return;
    setActionError("");
    setBusy(true);
    try {
      const latest = await api<ConversationData>(endpoint);
      setData(latest);
      const people = await api<{ items: { id: string; name: string }[] }>(
        `${base(workspace, "conversation-participants")}`,
      );
      const options = permissions.has("ai-agents.read")
        ? await api<{
            items: {
              id: string;
              name: string;
              state: string;
              visibility: string;
            }[];
          }>(`${base(workspace, "ai-agents")}?page_size=100`)
        : { items: [] };
      setMemberOptions(
        [...latest.members, ...people.items].filter(
          (v, i, a) => a.findIndex((x) => x.id === v.id) === i,
        ),
      );
      setAgentOptions(
        [
          ...latest.agents,
          ...options.items.filter(
            (a) => a.state === "enabled" && a.visibility === "workspace",
          ),
        ].filter((v, i, a) => a.findIndex((x) => x.id === v.id) === i),
      );
      setMemberIds(latest.members.map((m) => m.id));
      setAgentIds(latest.agents.map((a) => a.id));
      setMembersVersion(latest.members_version);
      setMemberSearch("");
      setMemberPicker(true);
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  async function saveMembers() {
    setBusy(true);
    setActionError("");
    try {
      await api(endpoint + "/members", {
        method: "PUT",
        body: JSON.stringify({
          version: membersVersion,
          member_ids: memberIds,
          agent_ids: agentIds,
        }),
      });
      setData(await api<ConversationData>(endpoint));
      setTargets((v) => v.filter((id) => agentIds.includes(id)));
      setMentions((v) => v.filter((id) => memberIds.includes(id)));
      setMemberPicker(false);
    } catch (cause) {
      setActionError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  if (!data)
    return connection ? (
      <p role="alert">{connection}</p>
    ) : (
      <LoadingSkeleton variant="form" />
    );
  return (
    <section
      className="relative flex min-h-0 min-w-0 flex-1 flex-col"
      aria-label={t("title")}
    >
      <header className="shrink-0 border-b bg-background px-3 py-3 pl-12 sm:px-6">
        <div className="flex items-center justify-between gap-3">
          <button
            type="button"
            className="flex min-w-0 items-center gap-3 rounded-md text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
            onClick={() => setInfoOpen(true)}
          >
            <span
              aria-hidden
              className="hidden size-10 shrink-0 items-center justify-center rounded-lg bg-muted sm:flex"
            >
              {data.kind === "group" ? (
                <Users className="size-5" />
              ) : (
                <Bot className="size-5" />
              )}
            </span>
            <div className="min-w-0">
              <h2 className="truncate text-base font-semibold">{data.title}</h2>
              <p className="mt-0.5 truncate text-xs text-muted-foreground">
                {data.kind === "group"
                  ? `${t("group")} · ${data.members.length + data.agents.length} ${t("participants")}`
                  : `${data.config.name} · ${data.config.model || t("nativeModel")}`}
              </p>
            </div>
          </button>
          <div className="flex shrink-0 items-center gap-1">
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("searchMessages")}
              aria-pressed={searchOpen}
              onClick={() => {
                setSearchOpen(!searchOpen);
                setSearch("");
              }}
            >
              <Search />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label={t(
                data.kind === "group"
                  ? "conversationMembers"
                  : "conversationInfo",
              )}
              onClick={() => setInfoOpen(true)}
            >
              {data.kind === "group" ? <Users /> : <Info />}
            </Button>
            {data.can_manage && headerActions}
            {data.can_manage && (
              <Button
                variant="ghost"
                size="icon"
                aria-label={t("deleteConversation")}
                disabled={data.turns.some((turn) => active(turn.state))}
                onClick={() => setConfirmDelete(true)}
              >
                <Trash2 />
              </Button>
            )}
          </div>
        </div>
        {searchOpen && (
          <div className="mt-3 flex items-center gap-2">
            <Input
              autoFocus
              aria-label={t("searchMessages")}
              placeholder={t("searchMessages")}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("close")}
              onClick={() => {
                setSearchOpen(false);
                setSearch("");
              }}
            >
              <X />
            </Button>
          </div>
        )}
      </header>
      <div
        className="flex shrink-0 flex-wrap items-center gap-1 border-b px-3 py-1.5 sm:px-6"
        role="group"
        aria-label={t("messageFilters")}
      >
        {["allMessages", "unread", "mentions", "saved"].map((value) => (
          <Button
            key={value}
            variant={activity === value ? "secondary" : "ghost"}
            size="sm"
            aria-pressed={activity === value}
            onClick={() => {
              setActivity(value);
              setThread("");
            }}
          >
            {t(value)}
            {value === "unread" && data.messages.some((m) => m.unread)
              ? ` · ${data.messages.filter((m) => m.unread).length}`
              : ""}
          </Button>
        ))}
        {activity === "unread" && data.messages.some((m) => m.unread) && (
          <Button
            className="ml-auto"
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() =>
              void act("/read", { message_id: data.messages.at(-1)?.id })
            }
          >
            {t("markRead")}
          </Button>
        )}
      </div>
      {thread && (
        <div className="flex shrink-0 items-center gap-2 border-b bg-muted/30 px-4 py-2 text-sm">
          <MessageSquare className="size-4" />
          <span className="shrink-0 font-medium">{t("thread")}</span>
          <span className="min-w-0 flex-1 truncate text-muted-foreground">
            {data.messages.find((m) => m.id === thread)?.prompt ||
              t("retracted")}
          </span>
          <Button
            size="icon"
            variant="ghost"
            aria-label={t("closeThread")}
            onClick={() => setThread("")}
          >
            <X />
          </Button>
        </div>
      )}
      <Dialog.Root open={infoOpen} onOpenChange={setInfoOpen}>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
          <Dialog.Content
            aria-describedby={undefined}
            className="fixed left-1/2 top-1/2 z-50 max-h-[85dvh] w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 overflow-auto rounded-lg border bg-background p-5 shadow-lg"
          >
            <div className="flex items-center justify-between gap-3">
              <Dialog.Title className="font-semibold">
                {t("conversationInfo")}
              </Dialog.Title>
              <Dialog.Close asChild>
                <Button variant="ghost" size="icon" aria-label={t("close")}>
                  <X />
                </Button>
              </Dialog.Close>
            </div>
            <p className="mt-3 font-medium">{data.title}</p>
            <h3 className="mt-5 mb-2 text-sm font-medium">
              {t("conversationMembers")}
            </h3>
            <ul className="space-y-2 text-sm">
              {data.members.map((member) => (
                <li key={member.id} className="flex items-center gap-2">
                  <User className="size-4 text-muted-foreground" />
                  {member.name}
                </li>
              ))}
              {(data.kind === "group"
                ? data.agents
                : [{ id: "agent", name: data.config.name, state: "active" }]
              ).map((agent) => (
                <li key={agent.id} className="flex items-center gap-2">
                  <Bot className="size-4 text-muted-foreground" />
                  {agent.name}
                </li>
              ))}
            </ul>
            {data.kind === "group" && data.can_manage && (
              <Button
                className="mt-4"
                variant="outline"
                disabled={busy || data.turns.some((turn) => active(turn.state))}
                onClick={() => void openMembers()}
              >
                {t("manageMembers")}
              </Button>
            )}
            <details className="mt-5 border-t pt-3 text-xs text-muted-foreground">
              <summary className="cursor-pointer">
                {t("executionDetails")}
              </summary>
              <dl className="mt-3 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-2">
                <dt>{t("runtime")}</dt>
                <dd>
                  {data.config.runtime === "claude"
                    ? "Claude Code"
                    : data.config.runtime === "codex"
                      ? "Codex"
                      : data.config.runtime}{" "}
                  · {data.config.model || t("nativeModel")}
                </dd>
                <dt>{t("revision")}</dt>
                <dd className="break-all">{data.revision_id}</dd>
                <dt>{t("directory")}</dt>
                <dd className="break-all">
                  {data.binding.canonical_directory}
                </dd>
              </dl>
            </details>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
      {connection && (
        <p role="status" className="text-sm text-destructive">
          {t("reconnecting")} {connection}
        </p>
      )}
      <div
        ref={scroll}
        onScroll={() => {
          const node = scroll.current!;
          follow.current =
            node.scrollHeight - node.scrollTop - node.clientHeight < 80;
          if (follow.current) setNewBelow(false);
        }}
        className="min-h-0 flex-1 overflow-auto overscroll-contain px-2 pb-6 pt-3 sm:px-6"
        aria-label={t("messages")}
      >
        <div className="mx-auto w-full max-w-4xl">
          {activity !== "allMessages" &&
            !data.messages.some((m) =>
              activity === "unread"
                ? m.unread
                : activity === "saved"
                  ? m.saved
                  : m.mentions?.includes(data.viewer_id),
            ) && (
              <p
                role="status"
                className="py-12 text-center text-sm text-muted-foreground"
              >
                {t("noFilteredMessages")}
              </p>
            )}
          {!data.turns.length && !data.messages?.length && (
            <p className="py-8 text-sm text-muted-foreground">
              {t("noMessages")}
            </p>
          )}
          {conversationTimeline(data.messages, data.turns)
            .filter((entry) => {
              const m = entry.message;
              if (thread) {
                let id = m?.id;
                const seen = new Set<string>();
                while (id && !seen.has(id)) {
                  if (id === thread) return true;
                  seen.add(id);
                  id =
                    data.messages.find((v) => v.id === id)?.reply_to ??
                    undefined;
                }
                return (
                  !m &&
                  entry.turn &&
                  data.messages.some(
                    (v) =>
                      v.id === thread &&
                      (v.turn_id === entry.turn?.id ||
                        v.id === entry.turn?.message_id),
                  )
                );
              }
              if (activity === "unread") return !!m?.unread;
              if (activity === "saved") return !!m?.saved;
              if (activity === "mentions")
                return !!m?.mentions?.includes(data.viewer_id);
              return true;
            })
            .filter(
              (entry) =>
                !search.trim() ||
                (entry.message?.prompt ?? entry.turn?.prompt ?? "")
                  .toLocaleLowerCase()
                  .includes(search.trim().toLocaleLowerCase()),
            )
            .map((entry, index, entries) => {
              const m = entry.message;
              const turn = entry.turn;
              const own = m?.role === "user" && m.owner_id === data.viewer_id;
              const name =
                m?.role === "user"
                  ? m.sender_name ||
                    data.members.find((member) => member.id === m.owner_id)
                      ?.name ||
                    t("member")
                  : m?.sender_name ||
                    (data.kind === "group"
                      ? data.agents.find(
                          (agent) =>
                            agent.id === (m?.agent_id ?? turn?.agent_id),
                        )?.name
                      : data.config.name) ||
                    t("agent");
              const date = entry.created_at ? new Date(entry.created_at) : null;
              const previous = entries[index - 1];
              const newDay =
                date &&
                (!previous ||
                  new Date(previous.created_at).toDateString() !==
                    date.toDateString());
              const reply = m?.role === "assistant" ? m : undefined;
              const items = new Map<
                string,
                {
                  text?: string | undefined;
                  name?: string | undefined;
                  state?: string | undefined;
                }
              >();
              for (const { event } of events.filter(
                (event) => m?.role !== "user" && event.turn_id === turn?.id,
              )) {
                if (event.type === "tool" && event.item_id)
                  items.set(event.item_id, {
                    name: event.name,
                    state: event.state,
                  });
              }
              return (
                <Fragment key={m?.id ?? `turn-${turn!.id}`}>
                  {newDay && (
                    <div
                      className="flex items-center gap-3 py-4"
                      role="separator"
                    >
                      <div className="h-px flex-1 bg-border" />
                      <time
                        className="shrink-0 text-xs text-muted-foreground"
                        dateTime={date.toISOString()}
                      >
                        {date.toLocaleDateString(i18n.language, {
                          year: "numeric",
                          month: "long",
                          day: "numeric",
                        })}
                      </time>
                      <div className="h-px flex-1 bg-border" />
                    </div>
                  )}
                  <article
                    data-message-id={m?.id}
                    data-turn-id={m?.role === "user" ? undefined : turn?.id}
                    className="group/message relative flex min-w-0 flex-wrap gap-x-3 gap-y-2 rounded-lg px-2 py-4 hover:bg-muted/35 sm:px-3"
                  >
                    <div
                      aria-hidden
                      className={`mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md ${m?.role === "user" ? "bg-muted text-muted-foreground" : "bg-primary/10 text-primary"}`}
                    >
                      {m?.role === "user" ? (
                        <User className="size-4" />
                      ) : (
                        <Bot className="size-4" />
                      )}
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="mb-1 flex min-h-6 flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
                        <span className="font-semibold text-foreground">
                          {name}
                        </span>
                        {m?.role === "assistant" && (
                          <span className="rounded border px-1 text-[10px]">
                            {t("agent")}
                          </span>
                        )}
                        {own && <span>{t("you")}</span>}
                        {!!m?.edited_at && <span>{t("edited")}</span>}
                        {date && (
                          <time
                            title={date.toLocaleString(i18n.language)}
                            dateTime={date.toISOString()}
                          >
                            {date.toLocaleTimeString(i18n.language, {
                              hour: "2-digit",
                              minute: "2-digit",
                            })}
                          </time>
                        )}
                      </div>
                      {m?.retracted_at ? (
                        <p className="py-1 text-sm italic text-muted-foreground">
                          {t("retracted")}
                        </p>
                      ) : (
                        m?.role === "user" && (
                          <div className="space-y-2">
                            {m.reply_to && (
                              <button
                                type="button"
                                className="block max-w-full truncate border-l-2 px-3 text-left text-xs text-muted-foreground hover:text-foreground"
                                onClick={() => {
                                  setThread(m.reply_to!);
                                  setActivity("allMessages");
                                }}
                              >
                                {t("reply")} ·{" "}
                                {data.messages.find((v) => v.id === m.reply_to)
                                  ?.prompt || t("retracted")}
                              </button>
                            )}
                            <p className="whitespace-pre-wrap break-words text-sm leading-7">
                              {m.prompt}
                            </p>
                            {!!m.targets.length && (
                              <p className="text-xs text-primary">
                                {m.targets
                                  .map(
                                    (id) =>
                                      "@" +
                                      (data.agents.find(
                                        (agent) => agent.id === id,
                                      )?.name ?? t("agent")),
                                  )
                                  .join(" ")}
                              </p>
                            )}
                            {!!m.mentions?.length && (
                              <p className="text-xs text-primary">
                                {m.mentions
                                  .map(
                                    (id) =>
                                      "@" +
                                      (data.members.find((p) => p.id === id)
                                        ?.name ?? t("member")),
                                  )
                                  .join(" ")}
                              </p>
                            )}
                            {!!m.attachments.length && (
                              <div className="grid max-w-xl gap-2">
                                {m.attachments.map((attachment, index) => (
                                  <AttachmentView
                                    key={attachment.file_id ?? index}
                                    attachment={attachment}
                                    endpoint={endpoint}
                                  />
                                ))}
                              </div>
                            )}
                          </div>
                        )
                      )}
                      <div className="space-y-3">
                        {reply && !reply.retracted_at && (
                          <div
                            data-agent-text
                            className="min-w-0 text-sm leading-7 break-words [&_pre]:my-3 [&_pre]:overflow-auto [&_pre]:rounded-lg [&_pre]:bg-muted [&_pre]:p-3 [&_pre]:text-xs [&_p]:my-2 [&_p:first-child]:mt-0 [&_h1]:mb-3 [&_h1]:mt-6 [&_h1]:text-xl [&_h1]:font-semibold [&_h2]:mb-2 [&_h2]:mt-5 [&_h2]:text-lg [&_h2]:font-semibold [&_h3]:mb-2 [&_h3]:mt-4 [&_h3]:font-semibold [&_blockquote]:my-3 [&_blockquote]:border-l-2 [&_blockquote]:pl-4 [&_blockquote]:text-muted-foreground [&_hr]:my-4 [&_ul]:list-disc [&_ul]:pl-5 [&_ol]:list-decimal [&_ol]:pl-5 [&_a]:text-primary [&_a]:underline [&_table]:block [&_table]:overflow-auto [&_th]:border [&_th]:p-2 [&_td]:border [&_td]:p-2"
                          >
                            <ReactMarkdown
                              remarkPlugins={[remarkGfm]}
                              components={{
                                img: ({ alt }) => <span>{alt}</span>,
                              }}
                            >
                              {reply.prompt}
                            </ReactMarkdown>
                          </div>
                        )}
                        {!!items.size && (
                          <details className="text-xs text-muted-foreground">
                            <summary className="cursor-pointer">
                              {t("executionDetails")}
                            </summary>
                            <div className="mt-2 space-y-1">
                              {[...items].map(([key, item]) => (
                                <p key={key}>
                                  {item.name} · {item.state}
                                </p>
                              ))}
                            </div>
                          </details>
                        )}
                        {turn && m?.role !== "user" && (
                          <>
                            {Object.entries(turn.decisions).map(
                              ([request, decision]) => (
                                <div
                                  key={request}
                                  className="space-y-2 rounded-md border p-3"
                                >
                                  <p className="font-medium">{decision.tool}</p>
                                  <pre className="max-h-56 overflow-auto whitespace-pre-wrap break-all text-xs">
                                    {JSON.stringify(decision.input, null, 2)}
                                  </pre>
                                  {decision.decision === null &&
                                  turn.state === "running" &&
                                  runnable &&
                                  turn.owner_id === data.viewer_id ? (
                                    <div className="flex gap-2">
                                      <Button
                                        size="sm"
                                        disabled={busy}
                                        onClick={() =>
                                          void act(
                                            `/turns/${turn.id}/decisions`,
                                            {
                                              request_id: request,
                                              allow: true,
                                            },
                                          )
                                        }
                                      >
                                        {t("allow")}
                                      </Button>
                                      <Button
                                        size="sm"
                                        variant="outline"
                                        disabled={busy}
                                        onClick={() =>
                                          void act(
                                            `/turns/${turn.id}/decisions`,
                                            {
                                              request_id: request,
                                              allow: false,
                                            },
                                          )
                                        }
                                      >
                                        {t("deny")}
                                      </Button>
                                    </div>
                                  ) : (
                                    <p className="text-sm text-muted-foreground">
                                      {decision.sent
                                        ? t("decisionSent")
                                        : decision.decision !== null
                                          ? t("decisionQueued")
                                          : t("decisionExpired")}
                                    </p>
                                  )}
                                </div>
                              ),
                            )}
                            {active(turn.state) && (
                              <p
                                role="status"
                                className="flex items-center gap-2 text-xs text-muted-foreground"
                              >
                                <span className="size-1.5 animate-pulse rounded-full bg-current" />
                                {t(turn.state)}
                              </p>
                            )}
                            {turn.usage && (
                              <details className="text-xs text-muted-foreground">
                                <summary className="cursor-pointer">
                                  {t("executionDetails")}
                                </summary>
                                <p className="mt-2">
                                  {t(turn.state)} · {t("inputTokens")}:{" "}
                                  {turn.usage.input_tokens} ·{" "}
                                  {t("outputTokens")}:{" "}
                                  {turn.usage.output_tokens}
                                </p>
                              </details>
                            )}
                            {turn.error && (
                              <p className="text-sm text-destructive">
                                {turn.error}
                              </p>
                            )}
                          </>
                        )}
                      </div>
                    </div>
                    {m && !m.retracted_at && (
                      <div className="ml-11 flex w-[calc(100%-2.75rem)] items-center text-muted-foreground sm:absolute sm:right-2 sm:top-0 sm:ml-0 sm:w-auto sm:rounded-md sm:border sm:bg-background sm:p-0.5 sm:shadow-sm sm:opacity-0 sm:group-hover/message:opacity-100 sm:group-focus-within/message:opacity-100">
                        {m.prompt && <CopyMessage text={m.prompt} />}
                        <Button
                          type="button"
                          size="icon"
                          variant="ghost"
                          className="size-7"
                          aria-label={t("reply")}
                          title={t("reply")}
                          disabled={!runnable}
                          onClick={() => {
                            setThread(m.id);
                            setActivity("allMessages");
                            follow.current = true;
                            requestAnimationFrame(() =>
                              document
                                .querySelector<HTMLTextAreaElement>("textarea")
                                ?.focus(),
                            );
                          }}
                        >
                          <MessageSquare className="size-3.5" />
                        </Button>
                        <Button
                          type="button"
                          size="icon"
                          variant="ghost"
                          className="size-7"
                          aria-label={t(
                            m.saved ? "unsaveMessage" : "saveMessage",
                          )}
                          title={t(m.saved ? "unsaveMessage" : "saveMessage")}
                          aria-pressed={!!m.saved}
                          disabled={busy}
                          onClick={() =>
                            void mutateMessage(
                              m,
                              m.saved ? "DELETE" : "PUT",
                              "/saved",
                            )
                          }
                        >
                          <Bookmark
                            className={`size-3.5 ${m.saved ? "fill-current" : ""}`}
                          />
                        </Button>
                        {own && runnable && (
                          <>
                            <Button
                              type="button"
                              size="icon"
                              variant="ghost"
                              className="size-7"
                              aria-label={t("editMessage")}
                              title={t("editMessage")}
                              disabled={
                                busy || data.turns.some((v) => active(v.state))
                              }
                              onClick={() => {
                                setEditDraft(m.prompt);
                                setActionError("");
                                setMessageAction({
                                  message: m,
                                  action: "edit",
                                });
                              }}
                            >
                              <Pencil className="size-3.5" />
                            </Button>
                            <Button
                              type="button"
                              size="icon"
                              variant="ghost"
                              className="size-7"
                              aria-label={t("retractMessage")}
                              title={t("retractMessage")}
                              disabled={
                                busy || data.turns.some((v) => active(v.state))
                              }
                              onClick={() => {
                                setActionError("");
                                setMessageAction({
                                  message: m,
                                  action: "retract",
                                });
                              }}
                            >
                              <Undo2 className="size-3.5" />
                            </Button>
                          </>
                        )}
                      </div>
                    )}
                  </article>
                </Fragment>
              );
            })}
        </div>
      </div>
      {newBelow && !search && (
        <Button
          variant="secondary"
          size="sm"
          className="shrink-0 self-center shadow-sm"
          onClick={() => {
            follow.current = true;
            setNewBelow(false);
            scroll.current?.scrollTo({
              top: scroll.current.scrollHeight,
              behavior: "smooth",
            });
          }}
        >
          <ArrowDown className="size-3.5" />
          {t("latestMessages")}
        </Button>
      )}
      {search &&
        !data.messages.some((message) =>
          message.prompt
            .toLocaleLowerCase()
            .includes(search.trim().toLocaleLowerCase()),
        ) && (
          <p
            role="status"
            className="py-3 text-center text-sm text-muted-foreground"
          >
            {t("noSearchResults")}
          </p>
        )}
      {data.state !== "active" && (
        <p role="status" className="text-sm text-muted-foreground">
          {t(data.state === "interrupted" ? "interruptedHelp" : "archived")}
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <div className="shrink-0 px-3 pb-3 pt-2 sm:px-6 sm:pb-5">
        <Composer
          endpoint={endpoint}
          draft={{
            prompt: draft,
            attachments,
            targets,
            mentions,
            reply_to: thread,
          }}
          onChange={(value) => {
            setDraft(value.prompt);
            setAttachments(value.attachments);
            setTargets(value.targets ?? []);
            setMentions(value.mentions ?? []);
          }}
          onSend={send}
          disabled={!runnable}
          busy={busy}
          current={current}
          group={data.kind === "group"}
          agents={data.agents ?? []}
          people={data.members.filter((m) => m.id !== data.viewer_id)}
          onStop={() => {
            if (current) void act(`/turns/${current.id}/cancellations`);
          }}
        />
      </div>
      <Dialog.Root
        open={!!messageAction}
        onOpenChange={(open) => {
          if (!open && !busy) setMessageAction(undefined);
        }}
      >
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
          <Dialog.Content
            aria-describedby={undefined}
            className="fixed left-1/2 top-1/2 z-50 w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 space-y-4 rounded-lg border bg-background p-5 shadow-lg"
          >
            <Dialog.Title className="font-semibold">
              {t(
                messageAction?.action === "edit"
                  ? "editMessage"
                  : "retractMessage",
              )}
            </Dialog.Title>
            <p className="text-sm text-muted-foreground">
              {t(
                messageAction?.action === "edit"
                  ? "editHistoryHelp"
                  : "retractConfirm",
              )}
            </p>
            {messageAction?.action === "edit" && (
              <textarea
                aria-label={t("editMessage")}
                className="min-h-36 w-full resize-y rounded-md border bg-background p-3 text-sm"
                value={editDraft}
                maxLength={48000}
                disabled={busy}
                onChange={(e) => setEditDraft(e.target.value)}
              />
            )}
            {actionError && (
              <p role="alert" className="text-sm text-destructive">
                {actionError}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                disabled={busy}
                onClick={() => setMessageAction(undefined)}
              >
                {t("close")}
              </Button>
              <Button
                variant={
                  messageAction?.action === "retract"
                    ? "destructive"
                    : "default"
                }
                disabled={
                  busy ||
                  (messageAction?.action === "edit" && !editDraft.trim())
                }
                onClick={() => {
                  if (messageAction)
                    void mutateMessage(
                      messageAction.message,
                      messageAction.action === "edit" ? "PATCH" : "DELETE",
                      "",
                      {
                        version: messageAction.message.version,
                        prompt:
                          messageAction.action === "edit"
                            ? editDraft
                            : undefined,
                      },
                    );
                }}
              >
                {t(
                  messageAction?.action === "edit"
                    ? "saveChanges"
                    : "retractMessage",
                )}
              </Button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
      <Dialog.Root
        open={memberPicker}
        onOpenChange={(open) => {
          if (!busy) setMemberPicker(open);
        }}
      >
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
          <Dialog.Content
            aria-describedby={undefined}
            className="fixed left-1/2 top-1/2 z-50 flex max-h-[85dvh] w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-lg border bg-background p-5 shadow-lg"
          >
            <Dialog.Title className="font-semibold">
              {t("manageMembers")}
            </Dialog.Title>
            <Input
              aria-label={t("memberSearch")}
              placeholder={t("memberSearch")}
              value={memberSearch}
              onChange={(e) => setMemberSearch(e.target.value)}
            />
            <div className="min-h-0 space-y-4 overflow-auto">
              {[
                {
                  label: "people",
                  options: memberOptions,
                  ids: memberIds,
                  set: setMemberIds,
                },
                {
                  label: "agent",
                  options: agentOptions,
                  ids: agentIds,
                  set: setAgentIds,
                },
              ].map(({ label, options, ids, set }) => (
                <fieldset key={label} className="space-y-1">
                  <legend className="mb-2 text-sm font-medium">
                    {t(label)}
                  </legend>
                  {options
                    .filter((p) =>
                      p.name
                        .toLocaleLowerCase()
                        .includes(memberSearch.toLocaleLowerCase()),
                    )
                    .map((p) => (
                      <label
                        key={p.id}
                        className="flex min-h-10 items-center gap-3 rounded-md px-2 text-sm hover:bg-muted"
                      >
                        <input
                          type="checkbox"
                          checked={ids.includes(p.id)}
                          disabled={
                            busy ||
                            (label === "people" && p.id === data.owner_id)
                          }
                          onChange={(e) =>
                            set(
                              e.target.checked
                                ? [...ids, p.id]
                                : ids.filter((id) => id !== p.id),
                            )
                          }
                        />
                        {p.name}
                      </label>
                    ))}
                </fieldset>
              ))}
            </div>
            {actionError && (
              <p role="alert" className="text-sm text-destructive">
                {actionError}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                disabled={busy}
                onClick={() => setMemberPicker(false)}
              >
                {t("close")}
              </Button>
              <Button disabled={busy} onClick={() => void saveMembers()}>
                {t("saveChanges")}
              </Button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
      <AlertDialog.Root open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialog.Portal>
          <AlertDialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
          <AlertDialog.Content className="fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 space-y-4 rounded-lg border bg-background p-5 shadow-lg">
            <AlertDialog.Title className="font-semibold">
              {t("deleteConversation")}
            </AlertDialog.Title>
            <AlertDialog.Description className="text-sm text-muted-foreground">
              {t("deleteHelp")}
            </AlertDialog.Description>
            <div className="flex justify-end gap-2">
              <AlertDialog.Cancel asChild>
                <Button variant="outline" disabled={deleting}>
                  {t("close")}
                </Button>
              </AlertDialog.Cancel>
              <Button
                variant="destructive"
                disabled={deleting}
                onClick={() => void removeConversation()}
              >
                {t("deleteConversation")}
              </Button>
            </div>
          </AlertDialog.Content>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    </section>
  );
}
