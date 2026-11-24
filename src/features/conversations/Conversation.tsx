import { useWorkspaceTimezone, formatDate, dayKey } from "../../shared/timezone";
import { ConversationMembers, type ConversationMember } from "./ConversationMembers";
import { normalizeRoom } from "./room-agents";
import { HeldRelays, RoomControls } from "./room-controls";
import { InlineAlert } from "../../shared/ui/inline-alert";
import { cliToolLabel } from "../../shared/forms/cli-tools";
import { randomUUID } from "../../shared/uuid";
import { IdentityAvatar, type AvatarValue } from "../../shared/ui/identity-avatar";
import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ApiError, api, errorText } from "../../shared/api/api";
import { canResumeSession } from "../../shared/api/session";
import { watchPageResume } from "../../shared/api/use-auto-refresh";
import { base } from "../../shared/api/resource";
import { AlertDialog, Dialog } from "radix-ui";
import { toast } from "sonner";
import {
  Trash2,
  Bot,
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
  RotateCcw,
  Ellipsis,
} from "lucide-react";
import { ConversationMessage } from "./ConversationMarkdown";
import { CollapsibleMessage } from "./CollapsibleMessage";
import { ConversationAppearance, conversationBackground, type ChatAppearance } from "./ConversationAppearance";
import { AttachmentView, type Attachment } from "./AttachmentView";
import { Composer, type ComposerDraft } from "./Composer";
import { commonText, i18n, useCommonTranslation, workflowText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { conversationText as t } from "./resource-config";
import {
  awaitingDecision,
  EndedDecisionCard,
  endedDecision,
  ExecutionTimeline,
  ExecutionDetailsPanel,
  QuestionCard,
  ConversationApproval,
  liveDecision,
  PendingDecisionCard,
  timelineSummary,
  type ActivityEvent,
  type Decision,
} from "./TurnActivity";
export type ConversationDraft = ComposerDraft;
interface Turn {
  id: string;
  owner_id: string;
  message_id?: string;
  purpose?: "message" | "introduction";
  introduction_id?: string;
  can_retry_introduction?: boolean;
  introduction_retry_warning?: string | null;
  introduction_retry_reason?: string | null;
  agent_id?: string;
  room_agent_id?: string;
  hop?: number;
  state: string;
  prompt: string;
  attachments: Attachment[];
  error: string;
  /** Identity of the error message, for the action that answers it. */
  error_key?: string | null;
  usage: { input_tokens: number; output_tokens: number } | null;
  decisions: Record<string, Decision>;
  created_at?: number;
  updated_at?: number;
}
interface BoundHost {
  id: string;
  name: string;
}
interface ChatMessage {
  id: string;
  /** "notice" lines come from the platform itself, e.g. a host switch. */
  /** "relay" is one room Agent writing to others. */
  role: "user" | "assistant" | "notice" | "relay";
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
  agent_id: string;
  agent_avatar?: AvatarValue;
  /** False when the Agent has no configured model and therefore cannot run. */
  agent_has_model?: boolean;
  members: { id: string; name: string; avatar?: AvatarValue }[];
  agents: { id: string; name: string; state: string; avatar?: AvatarValue; runtime?: string; model?: string; revision_id?: string; host?: BoundHost | null; room_agent_id?: string; agent_id?: string; added_by?: string; sponsor_name?: string; approval_mode?: string }[];
  messages: ChatMessage[];
  id: string;
  title: string;
  state: string;
  revision_id: string;
  config: { name: string; runtime: string; model: string; cwd: string; appearance?: ChatAppearance; approval_mode?: string; relay_enabled?: boolean };
  /** API execution has no binding; CLI execution binds at its first message. */
  binding: { execution_user: string; canonical_directory: string } | null;
  host?: BoundHost | null;
  turns: Turn[];
  version: number;
  owner_id: string;
  members_version: string;
}
const fetchConversation = (endpoint: string, init?: RequestInit) =>
  api<ConversationData>(endpoint, init).then(row => normalizeRoom(row));
interface Event {
  sequence: number;
  turn_id: string;
  event: ActivityEvent & { text?: string; replace?: boolean };
}
export interface ConversationSnapshot {
  id: string;
  data: ConversationData;
  events: Event[];
  cursor: number;
}
const active = (value: string) =>
  ["queued", "running", "stopping"].includes(value);
type ExecutionItem = { name?: string | undefined; state?: string | undefined; result?: unknown };
const NO_ITEMS: ReadonlyMap<string, ExecutionItem> = new Map();
const NO_EVENTS: readonly ActivityEvent[] = [];
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
      title={t(copied ? "copiedMessage" : "copyMessage")}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(text);
          setCopied(true);
          clearTimeout(timer.current);
          timer.current = setTimeout(() => setCopied(false), 1800);
        } catch {
          setCopied(false);
          toast.error(commonText("common.copyFailed"));
        }
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
  onOpenConversation,
  onPendingChange,
  initialSnapshot,
  onSnapshotChange,
  navigationAction,
  appActions,
}: {
  workspace: string;
  id: string;
  permissions: Set<string>;
  headerActions?: ReactNode;
  initialDraft?: ConversationDraft | undefined;
  onDraftChange?: (draft: ConversationDraft) => void;
  onDeleted?: () => void;
  onOpenConversation?: (id: string) => void;
  onPendingChange?: (id: string, count: number) => void;
  initialSnapshot?: ConversationSnapshot | undefined;
  onSnapshotChange?: (snapshot: ConversationSnapshot | undefined) => void;
  navigationAction?: ReactNode;
  appActions?: ReactNode;
}) {
  useCommonTranslation();
  const timezone = useWorkspaceTimezone();
  const endpoint = `${base(workspace, "conversations")}/${encodeURIComponent(id)}`;
  const restored = initialSnapshot?.id === id ? initialSnapshot : undefined;
  const [data, setData] = useState<ConversationData | undefined>(restored?.data);
  // Invalidate reads started before a decision write completed.
  const decisionRevision = useRef(0);
  const [events, setEvents] = useState<Event[]>(restored?.events ?? []);
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
  const [selectedMember, setSelectedMember] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const searchInput = useRef<HTMLInputElement>(null);
  const [search, setSearch] = useState("");
  const [newBelow, setNewBelow] = useState(false);
  const [error, setError] = useState("");
  const [connection, setConnection] = useState("");
  const [busy, setBusy] = useState(false);
  const pending = useRef<{ signature: string; client_id: string } | undefined>(
    undefined,
  );
  const cursor = useRef(restored?.cursor ?? 0);
  // The last conversation snapshot applied, so an unchanged poll renders nothing.
  const applied = useRef("");
  useEffect(() => {
    onSnapshotChange?.(data ? { id, data, events, cursor: cursor.current } : undefined);
  }, [id, data, events, onSnapshotChange]);
  // A private message typed while a reply runs is sent once that reply finishes.
  const [queue, setQueue] = useState<{ after: string; state: "waiting" | "paused" }>();
  const queueSent = useRef("");
  // Execution items per turn, built once per event change instead of per row.
  // Files each turn delivered on its reply, for the member activity summary.
  const turnFiles = useMemo(() => {
    const counts = new Map<string, number>();
    for (const message of data?.messages ?? []) {
      if (message.role === "assistant" && message.turn_id && message.attachments?.length)
        counts.set(message.turn_id, message.attachments.length);
    }
    return counts;
  }, [data?.messages]);
  const turnItems = useMemo(() => {
    const index = new Map<string, Map<string, ExecutionItem>>();
    for (const { turn_id, event } of events) {
      const key =
        event.type === "tool" ? event.item_id : event.type === "platform_result" ? event.request_id : undefined;
      if (!key) continue;
      let items = index.get(turn_id);
      if (!items) index.set(turn_id, (items = new Map()));
      items.set(
        key,
        event.type === "tool"
          ? { name: event.name, state: event.state }
          : { name: t("platformResult"), result: event.result },
      );
    }
    return index;
  }, [events]);
  // Raw execution events per turn, for the readable timeline.
  const turnEvents = useMemo(() => {
    const index = new Map<string, ActivityEvent[]>();
    for (const { turn_id, event } of events) {
      if (!["tool", "platform_result", "approval"].includes(event.type)) continue;
      let list = index.get(turn_id);
      if (!list) index.set(turn_id, (list = []));
      list.push(event);
    }
    return index;
  }, [events]);
  // Count actionable requests, matching the conversation list API.
  const attention = useMemo(() => {
    const turns = new Set<string>();
    let count = 0;
    for (const turn of data?.turns ?? []) {
      for (const decision of Object.values(turn.decisions)) {
        if (!awaitingDecision(turn, decision)) continue;
        turns.add(turn.id);
        count++;
      }
    }
    return { turns, count };
  }, [data?.turns]);
  useEffect(() => {
    if (data) onPendingChange?.(id, attention.count);
  }, [id, data, attention.count, onPendingChange]);
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
    let controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function refresh(controller: AbortController) {
      if (document.hidden) return;
      try {
        const revision = decisionRevision.current;
        const row = await fetchConversation(endpoint, {
          signal: controller.signal,
        });
        if (controller.signal.aborted) return;
        const snapshot = JSON.stringify(row);
        if (revision === decisionRevision.current && snapshot !== applied.current) {
          applied.current = snapshot;
          setData(row);
        }
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
          const fresh = result.items.filter((event) => event.sequence > previous);
          if (fresh.length) setEvents((old) => [...old, ...fresh]);
          more = result.has_more;
        }
        setConnection("");
      } catch (cause) {
        if (controller.signal.aborted) return;
        if (
          cause instanceof ApiError &&
          [401, 403, 404].includes(cause.status) &&
          !(cause.status === 401 && canResumeSession())
        ) {
          setData(undefined);
          setEvents([]);
          cursor.current = 0;
          applied.current = "";
        }
        setConnection(errorText(cause));
      } finally {
        if (!controller.signal.aborted && !document.hidden)
          timer = setTimeout(() => void refresh(controller), 1000);
      }
    }
    applied.current = "";
    const resume = () => {
      controller.abort();
      clearTimeout(timer);
      controller = new AbortController();
      void refresh(controller);
    };
    const stop = watchPageResume(resume);
    if (!document.hidden) void refresh(controller);
    return () => {
      stop();
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
  const recoverable = data?.kind === "private" && data.state === "interrupted" &&
    data.owner_id === data.viewer_id && permissions.has("conversations.update");
  const runnable = permissions.has("conversations.run") &&
    (data?.state === "active" || recoverable);
  useEffect(() => {
    if (!queue || queue.state !== "waiting" || !data || busy) return;
    if (data.turns.some((turn) => active(turn.state) && turn.owner_id === data.viewer_id)) return;
    const after = data.turns.find((turn) => turn.id === queue.after);
    if (after && after.state !== "completed") {
      setQueue({ ...queue, state: "paused" });
      return;
    }
    if (queueSent.current === queue.after) return;
    queueSent.current = queue.after;
    setQueue(undefined);
    if (draft.trim() || attachments.length) void send();
    // send reads the latest draft and data of this render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [queue, data, busy]);
  const lastTurn = data?.turns.at(-1);
  const introductionRetryClients = useRef(new Map<string, string>());
  const retryable = lastTurn && ["failed", "interrupted", "timed_out", "cancelled"].includes(lastTurn.state);
  function updateDecision(turnId: string, request: string, patch: Partial<Decision>) {
    setData(previous => previous ? { ...previous, turns: previous.turns.map(turn => turn.id === turnId ? {
      ...turn, decisions: { ...turn.decisions, [request]: { ...turn.decisions[request]!, ...patch } },
    } : turn) } : previous);
  }
  async function act(path: string, body: unknown = {}, committed?: () => void) {
    setError("");
    setBusy(true);
    try {
      await api(endpoint + path, {
        method: "POST",
        body: JSON.stringify(body),
      });
      const revision = ++decisionRevision.current;
      committed?.();
      const latest = await fetchConversation(endpoint);
      if (revision === decisionRevision.current) setData(latest);
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  async function retryIntroduction(turn: Turn) {
    const key = `${id}:${turn.id}`;
    const clientId = introductionRetryClients.current.get(key) ?? randomUUID();
    introductionRetryClients.current.set(key, clientId);
    await act(`/turns/${turn.id}/introduction-retries`, { client_id: clientId }, () => {
      introductionRetryClients.current.delete(key);
    });
  }
  /** Continue an approval left when its reply ended through the normal send path. */
  async function resolveEnded(turn: Turn, request: string, operation: string) {
    const sent = await send(undefined, {
      prompt: commonText("conversations:continuePrompt", { operation }),
      targets: data?.kind === "group" && turn.agent_id ? [turn.agent_id] : [],
    });
    if (!sent) return;
    await act(`/turns/${turn.id}/decision-resolutions`, { request_id: request, action: "resumed" });
  }
  async function send(retry?: Turn, followUp?: { prompt: string; targets: string[] }): Promise<boolean> {
    const prompt = followUp?.prompt ?? retry?.prompt ?? draft;
    const files = followUp ? [] : retry?.attachments ?? attachments;
    const replyTo = followUp ? undefined : retry ? data?.messages.find(message => message.role === "user" && message.turn_id === retry.id)?.reply_to : thread;
    const sendTargets = followUp ? followUp.targets : targets;
    const sendMentions = followUp ? [] : mentions;
    if ((!prompt.trim() && !files.length) || !runnable || busy) return false;
    if (current && data?.kind !== "group") {
      // Keep the draft; it is sent once this reply finishes.
      if (!retry && !followUp) setQueue({ after: current.id, state: "waiting" });
      return false;
    }
    const signature = JSON.stringify({
      prompt,
      attachments: files,
      retry: retry?.id,
      targets: sendTargets,
      mentions: sendMentions,
      thread: replyTo,
    });
    if (pending.current?.signature !== signature)
      pending.current = { signature, client_id: randomUUID() };
    setError("");
    setBusy(true);
    try {
      if (recoverable && data) {
        await api(endpoint, {
          method: "PATCH",
          body: JSON.stringify({title: data.title, state: "active", version: data.version}),
        });
      }
      await api(endpoint + (data?.kind === "group" ? "/messages" : "/turns"), {
        method: "POST",
        body: JSON.stringify({
          prompt,
          attachments: files,
          client_id: pending.current.client_id,
          ...(replyTo ? { reply_to: replyTo } : {}),
          ...(data?.kind === "group" ? { agents: sendTargets, mentions: sendMentions } : {}),
        }),
      });
      if (!retry && !followUp) {
        setDraft("");
        setAttachments([]);
        setTargets([]);
        setMentions([]);
        setThread("");
      }
      pending.current = undefined;
      follow.current = true;
      setData(await fetchConversation(endpoint));
      return true;
    } catch (cause) {
      setError(errorText(cause));
      // Recovery may have succeeded before a send/network failure. Keep the
      // server version fresh so the next explicit retry can reuse client_id.
      setData(await fetchConversation(endpoint).catch(() => data));
      return false;
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
      setData(await fetchConversation(endpoint));
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
      const latest = await fetchConversation(endpoint);
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
          ...latest.agents.filter((a) => a.added_by === undefined || a.added_by === latest.viewer_id).map((a) => ({ ...a, id: a.agent_id ?? a.id })),
          ...options.items.filter(
            (a) => a.state === "enabled" && a.visibility === "workspace",
          ),
        ].filter((v, i, a) => a.findIndex((x) => x.id === v.id) === i),
      );
      setMemberIds(latest.members.map((m) => m.id));
      setAgentIds(latest.agents.filter((a) => a.added_by === undefined || a.added_by === latest.viewer_id).map((a) => a.agent_id ?? a.id));
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
      const reloaded = await fetchConversation(endpoint);
      setData(reloaded);
      setTargets((v) => v.filter((id) => reloaded.agents.some((agent) => agent.id === id)));
      setMentions((v) => v.filter((id) => memberIds.includes(id)));
      setMemberPicker(false);
    } catch (cause) {
      setActionError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  if (!data) return <>
    <div className="flex shrink-0 items-center gap-2 border-b px-2 py-1 md:hidden">{navigationAction}<div className="ml-auto flex min-w-0 flex-wrap items-center gap-1">{appActions}</div></div>
    {connection ? <p role="alert">{connection}</p> : <LoadingSkeleton variant="form" />}
  </>;
  const managementActions = <>
            <ConversationApproval showLabel endpoint={endpoint} mode={data.config.approval_mode??"request"} version={data.version} canManage={!!data.can_manage&&permissions.has("conversations.update")} executing={executing} onSaved={(mode,version)=>setData(old=>old?{...old,version,config:{...old.config,approval_mode:mode}}:old)}/>
            {data.can_manage && permissions.has("conversations.update") && <ConversationAppearance showLabel endpoint={endpoint} value={data.config.appearance} canUpload={permissions.has("conversations.run") && data.state === "active"} onSaved={appearance => setData(old => old ? { ...old, config: { ...old.config, appearance } } : old)} />}
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("searchMessages")}
              aria-pressed={searchOpen}
              onClick={() => {
                setMoreOpen(false); setSearchOpen(!searchOpen);
                setSearch("");
              }}
            >
              <Search /><span data-conversation-action-label className="md:hidden">{t("searchMessages")}</span>
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label={t(
                data.kind === "group"
                  ? "conversationMembers"
                  : "conversationInfo",
              )}
              onClick={() => { setMoreOpen(false); setSelectedMember(""); setInfoOpen(true); }}
            >
              {data.kind === "group" ? <Users /> : <Info />}<span data-conversation-action-label className="md:hidden">{t(data.kind === "group" ? "conversationMembers" : "conversationInfo")}</span>
            </Button>
            {data.can_manage && <div onClick={() => setMoreOpen(false)}>{headerActions}</div>}
            {data.can_manage && (
              <Button
                variant="ghost"
                size="icon"
                aria-label={t("deleteConversation")}
                disabled={data.turns.some((turn) => active(turn.state))}
                onClick={() => { setMoreOpen(false); setConfirmDelete(true); }}
              >
                <Trash2 /><span data-conversation-action-label className="md:hidden">{t("deleteConversation")}</span>
              </Button>
            )}
  </>;
  const messageFilters = <>
        {["allMessages", "unread", "mentions", "saved", "pending"].map((value) => (
          <Button
            key={value}
            variant={activity === value ? "secondary" : "ghost"}
            size="sm"
            aria-pressed={activity === value}
            onClick={() => {
              setActivity(value); setMoreOpen(false);
              setThread("");
            }}
          >
            {t(value === "pending" ? "pendingFilter" : value)}
            {value === "unread" && data.messages.some((m) => m.unread)
              ? ` · ${data.messages.filter((m) => m.unread).length}`
              : ""}
            {value === "pending" && attention.count ? ` · ${attention.count}` : ""}
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
  </>;
  return (
    <section
      className="relative flex min-h-0 min-w-0 flex-1 flex-col"
      aria-label={t("title")}
    >
      <header data-conversation-header className="shrink-0 border-b bg-background px-3 py-2 md:px-6 md:py-3">
        <div className="flex items-center gap-1 md:flex-wrap md:justify-between md:gap-3">
          <div className="shrink-0 md:hidden" data-conversation-navigation>{navigationAction}</div>
          <button
            type="button"
            className="flex min-h-11 min-w-0 max-w-full flex-1 items-center gap-3 rounded-md text-left outline-none focus-visible:ring-2 focus-visible:ring-ring md:flex-none"
            onClick={() => { setSelectedMember(""); setInfoOpen(true); }}
          >
            <span
              aria-hidden
              className="hidden size-10 shrink-0 items-center justify-center rounded-lg bg-muted md:flex"
            >
              {data.kind === "group" ? (
                <Users className="size-5" />
              ) : (
                <Bot className="size-5" />
              )}
            </span>
            <div className="min-w-0">
              <h2 className="truncate text-base font-semibold">{data.title}</h2>
              {activity !== "allMessages" && <span className="block truncate text-xs text-muted-foreground md:hidden">{t(activity === "pending" ? "pendingFilter" : activity)}</span>}
              <p className="mt-0.5 hidden truncate text-xs text-muted-foreground md:block">
                {data.kind === "group"
                  ? `${t("group")} · ${data.members.length + data.agents.length} ${t("participants")}${data.agents.length ? ` · ${t("executionLocation")} ${data.agents.map(agent => `${agent.name}: ${agent.runtime === "api" ? workflowText("execution.server") : agent.host?.name || t("hostPending")}`).join(", ")}` : ""}`
                  : <>{data.config.name} · <ModelLabel model={data.config.model} hasModel={data.agent_has_model} /> · {data.config.runtime === "api" ? workflowText("execution.server") : <>{t("executionHost")} {data.host?.name || t("hostPending")}</>}</>}
              </p>
            </div>
          </button>
          <div className="hidden min-w-0 max-w-full flex-wrap items-center gap-1 md:flex">{managementActions}</div>
          <Dialog.Root open={moreOpen} onOpenChange={setMoreOpen}>
            <Dialog.Trigger asChild><Button data-conversation-more variant="ghost" size="icon" className="relative size-11 shrink-0 md:hidden" aria-label={t("conversationActions") + (attention.count ? ` · ${t("pendingFilter")} ${attention.count}` : "")}><Ellipsis />{attention.count > 0 && <span data-conversation-pending-indicator aria-hidden="true" className="absolute -right-1 top-0 min-w-4 rounded-full bg-primary px-1 text-[10px] leading-4 text-primary-foreground">{attention.count}</span>}</Button></Dialog.Trigger>
            <Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" /><Dialog.Content data-conversation-actions aria-describedby={undefined} className="dialog-shell fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-1.5rem))] -translate-x-1/2 -translate-y-1/2 rounded-2xl border bg-background p-4 shadow-xl" onCloseAutoFocus={event => { if (searchOpen) { event.preventDefault(); searchInput.current?.focus(); } else if (infoOpen || confirmDelete) event.preventDefault(); }}>
              <header className="flex items-center justify-between gap-2 pb-3"><Dialog.Title className="font-semibold">{t("conversationActions")}</Dialog.Title><Dialog.Close asChild><Button variant="ghost" size="icon" aria-label={commonText("common.close")}><X /></Button></Dialog.Close></header>
              <div className="dialog-body space-y-3">
                <div role="group" aria-label={t("messageFilters")} className="flex flex-wrap gap-1 border-b pb-3">{messageFilters}</div>
                <div className="grid gap-1 [&_button]:h-auto [&_button]:w-full [&_button]:justify-start [&_button]:gap-3 [&_button]:px-3 [&_button]:py-2 [&_button]:text-base">{managementActions}</div>
                {appActions && <div className="flex flex-wrap items-center gap-2 border-t pt-3">{appActions}</div>}
              </div>
            </Dialog.Content></Dialog.Portal>
          </Dialog.Root>
        </div>
        {searchOpen && (
          <div className="mt-3 flex items-center gap-2">
            <Input
              ref={searchInput}
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
      <div className="hidden shrink-0 flex-wrap items-center gap-1 border-b px-3 py-1.5 sm:px-6 md:flex" role="group" aria-label={t("messageFilters")}>{messageFilters}</div>
      <ConversationMembers open={infoOpen} onOpenChange={setInfoOpen} selected={selectedMember} onSelect={setSelectedMember}
        title={data.title} workspace={workspace} permissions={permissions} members={[
          ...data.members.map(member => ({ ...member, kind: "user" as const })),
          ...(data.kind === "group" ? data.agents : [{ id: data.agent_id, name: data.config.name, avatar: data.agent_avatar, runtime: data.config.runtime, model: data.config.model, revision_id: data.revision_id, host: data.host }]).map(agent => ({ ...agent, host: agent.runtime === "api" ? workflowText("execution.server") : agent.host?.name || t("hostPending"), kind: "agent" as const,
            ...("added_by" in agent && agent.added_by ? { sponsor: agent.sponsor_name, approvalMode: agent.approval_mode, canEditMode: agent.added_by === data.viewer_id, canRemove: agent.added_by === data.viewer_id || data.owner_id === data.viewer_id } : {}) })),
        ] satisfies ConversationMember[]} turns={data.turns} connected={!connection}
        activity={turnItems}
        files={turnFiles}
        onJumpToTurn={turnId => {
          setActivity("allMessages");
          requestAnimationFrame(() => [...(scroll.current?.querySelectorAll("[data-turn-id]") ?? [])]
            .find(node => node.getAttribute("data-turn-id") === turnId)
            ?.scrollIntoView({ block: "center", behavior: "smooth" }));
        }} ownerId={data.owner_id} viewerId={data.viewer_id} primaryAgentId={data.agent_id}
        onOpenConversation={onOpenConversation}
        roomControls={data.kind === "group" ? <RoomControls workspace={workspace} endpoint={endpoint} relayEnabled={data.config.relay_enabled !== false}
          canRun={permissions.has("conversations.run")} onChanged={async () => setData(await fetchConversation(endpoint))} /> : null}
        onApprovalMode={async (agentId, mode) => {
          await api(`${endpoint}/agents/${agentId}`, { method: "PATCH", body: JSON.stringify({ approval_mode: mode }) });
          setData(await fetchConversation(endpoint));
        }}
        onRemoveAgent={async agentId => {
          await api(`${endpoint}/agents/${agentId}`, { method: "DELETE" });
          setData(await fetchConversation(endpoint));
          setTargets(v => v.filter(id => id !== agentId));
        }}
        manage={data.kind === "group" && data.can_manage ? <Button variant="outline" disabled={busy || data.turns.some(turn => active(turn.state))} onClick={() => { setInfoOpen(false); void openMembers(); }}>{t("manageMembers")}</Button> : null}
        executionDetails={<>            <details className="mt-5 border-t pt-3 text-xs text-muted-foreground">
              <summary className="cursor-pointer">
                {t("executionDetails")}
              </summary>
              <dl className="mt-3 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-2">
                <dt>{t("runtime")}</dt>
                <dd>
                  {data.config.runtime === "api" ? "API" : cliToolLabel(data.config.runtime)}{" "}
                  · <ModelLabel model={data.config.model} hasModel={data.agent_has_model} />
                </dd>
                <dt>{t("revision")}</dt>
                <dd className="break-all">{data.revision_id}</dd>
                <dt>{t(data.config.runtime === "api" ? "executionLocation" : "executionHost")}</dt>
                <dd className="break-all">{data.config.runtime === "api" ? workflowText("execution.server") : data.host?.name || t("hostPending")}</dd>
                {data.config.runtime !== "api" && <><dt>{t("directory")}</dt>
                <dd className="break-all">
                  {data.binding?.canonical_directory || "—"}
                </dd></>}
              </dl>
            </details>
</>} />
      <div data-conversation-canvas className="relative flex min-h-0 flex-1 flex-col" style={conversationBackground(data.config.appearance, endpoint)}>
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
        className="conversation-feed min-h-0 flex-1 overflow-auto overscroll-contain px-2 pb-6 pt-3 sm:px-6"
        aria-label={t("messages")}
      >
        <div className={`mx-auto w-full max-w-4xl ${!data.turns.length && !data.messages.length ? "flex min-h-full flex-col" : ""}`}>
          {activity !== "allMessages" &&
            (activity === "pending"
              ? !attention.turns.size
              : !data.messages.some((m) =>
                  activity === "unread"
                    ? m.unread
                    : activity === "saved"
                      ? m.saved
                      : m.mentions?.includes(data.viewer_id),
                )) && (
              <p
                role="status"
                className="py-12 text-center text-sm text-muted-foreground"
              >
                {t("noFilteredMessages")}
              </p>
            )}
          {activity === "allMessages" && !data.turns.length && !data.messages.length && (
            <div data-conversation-empty className="flex flex-1 flex-col items-center justify-center px-4 py-8 text-center">
              <div className="flex max-w-sm flex-col items-center rounded-2xl bg-background/90 px-6 py-5 text-foreground shadow-sm">
              <svg aria-hidden="true" viewBox="0 0 160 120" className="mb-4 h-24 w-32 shrink-0 text-primary">
                <ellipse cx="80" cy="108" rx="47" ry="5" fill="currentColor" opacity=".06" />
                <path d="M47 19h59a17 17 0 0 1 17 17v30a17 17 0 0 1-17 17H77L54 99V83h-7a17 17 0 0 1-17-17V36a17 17 0 0 1 17-17Z" fill="currentColor" opacity=".09" />
                <path d="M47 19h59a17 17 0 0 1 17 17v30a17 17 0 0 1-17 17H77L54 99V83h-7a17 17 0 0 1-17-17V36a17 17 0 0 1 17-17Z" fill="none" stroke="currentColor" strokeWidth="1.7" opacity=".55" />
                <path d="M51 43h48M51 55h34M51 67h20" stroke="currentColor" strokeWidth="2" strokeLinecap="round" opacity=".4" />
                <path d="m113 29 27-12-9 28-6-10-12-6Z M125 35l15-18" fill="var(--background)" stroke="currentColor" strokeWidth="1.7" strokeLinejoin="round" />
                <path d="m17 54 2-5 2 5 5 2-5 2-2 5-2-5-5-2 5-2Z M130 78l2 5 5 2-5 2-2 5-2-5-5-2 5-2 2-5Z" fill="none" stroke="currentColor" strokeWidth="1.3" opacity=".4" />
              </svg>
              <h2 className="text-base font-semibold tracking-tight">{t("emptyConversationTitle")}</h2>
              <p className="mt-2 max-w-64 text-sm leading-relaxed text-muted-foreground">{t("noMessages")}</p>
              </div>
            </div>
          )}
          {conversationTimeline(data.messages, data.turns)
            .filter((entry) => {
              const m = entry.message;
              if (activity === "unread") return !!m?.unread;
              if (activity === "saved") return !!m?.saved;
              if (activity === "mentions")
                return !!m?.mentions?.includes(data.viewer_id);
              if (activity === "pending")
                return m?.role !== "user" && !!entry.turn && attention.turns.has(entry.turn.id);
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
              // A platform notice (e.g. the execution host changed) is a line, not a message bubble.
              if (m?.role === "notice")
                return (
                  <p key={m.id} role="note" data-conversation-notice className="mx-auto my-2 max-w-xl rounded-2xl border bg-card/90 px-3 py-1.5 text-center text-xs text-muted-foreground">
                    {m.prompt}
                  </p>
                );
              const own = m?.role === "user" && m.owner_id === data.viewer_id;
              const name =
                m?.role === "user"
                  ? m.sender_name ||
                    data.members.find((member) => member.id === m.owner_id)
                      ?.name ||
                    t("member")
                  : (m?.role === "relay" ? data.agents.find((agent) => agent.id === m.agent_id)?.name : undefined) ||
                    m?.sender_name ||
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
                  dayKey(previous.created_at, timezone, "device") !==
                    dayKey(date, timezone, "device"));
              const reply = m?.role === "assistant" ? m : undefined;
              const items =
                (m?.role !== "user" && turn ? turnItems.get(turn.id) : undefined) ?? NO_ITEMS;
              return (
                <Fragment key={m?.id ?? `turn-${turn!.id}`}>
                  {newDay && (
                    <div
                      className="flex items-center gap-3 py-3"
                      role="separator"
                    >
                      <div className="h-px flex-1 bg-border" />
                      <time
                        className="shrink-0 rounded-full border bg-card px-3 py-1 text-xs text-muted-foreground"
                        dateTime={date.toISOString()}
                      >
                        {formatDate(date, timezone, "date", i18n.language, {
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
                    className="group/message relative my-2 flex min-w-0 flex-wrap gap-x-2.5 gap-y-2 rounded-lg px-1 py-1.5 sm:px-3"
                  >
                    <button type="button" className="mt-0.5 shrink-0 rounded-full focus-visible:outline-2 focus-visible:outline-ring" aria-label={`${t("memberDetails")} · ${name}`}
                      onClick={() => { setSelectedMember(`${m?.role === "user" ? "user" : "agent"}:${m?.role === "user" ? m.owner_id : m?.agent_id || turn?.agent_id || data.agent_id}`); setInfoOpen(true); }}>
                      <IdentityAvatar id={m?.role === "user" ? m.owner_id : m?.agent_id || turn?.agent_id || data.agent_id} name={name} kind={m?.role === "user" ? "user" : "agent"}
                        value={m?.role === "user" ? data.members.find(member => member.id === m.owner_id)?.avatar : data.kind === "group" ? data.agents.find(agent => agent.id === (m?.agent_id || turn?.agent_id))?.avatar : data.agent_avatar} className="size-10" />
                    </button>
                    <div className={`min-w-[min(100%,12rem)] flex-1 rounded-xl border px-3 py-2.5 shadow-xs sm:px-4 ${m?.role === "user" ? "rounded-tl-sm border-primary/20 bg-muted text-foreground" : "rounded-tl-sm bg-card text-card-foreground"}`}>
                      <div className="mb-1 flex min-h-6 flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
                        <span className="font-semibold text-foreground">
                          {name}
                        </span>
                        {m?.role === "assistant" && (
                          <span className="rounded border px-1 text-[10px]">
                            {t("agent")}
                          </span>
                        )}
                        {m?.role === "relay" && (
                          <span data-relay-message className="rounded border border-primary/40 px-1 text-[10px] text-primary">
                            {t("relayMessage")}
                          </span>
                        )}
                        {own && <span>{t("you")}</span>}
                        {!!m?.edited_at && <span>{t("edited")}</span>}
                        {date && (
                          <time
                            title={formatDate(date, timezone, "datetime", i18n.language)}
                            dateTime={date.toISOString()}
                          >
                            {formatDate(date, timezone, "time", i18n.language, {
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
                        (m?.role === "user" || m?.role === "relay") && (
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
                            <CollapsibleMessage><p className="whitespace-pre-wrap break-words text-sm leading-relaxed">
                              {m.prompt}
                            </p></CollapsibleMessage>
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
                        {turn?.purpose === "introduction" && <p data-model-introduction className="text-xs text-muted-foreground">{t("modelIntroduction")}</p>}
                        {reply && !reply.retracted_at && (
                          <div
                            data-agent-text
                            className="min-w-0 text-sm leading-relaxed break-words"
                          >
                            <ConversationMessage streaming={!!turn && active(turn.state)} endpoint={endpoint}>
                              {reply.prompt}
                            </ConversationMessage>
                          </div>
                        )}
                        {reply && !reply.retracted_at && !!reply.attachments?.length && (
                          <div className="grid max-w-xl gap-2" data-agent-files>
                            {reply.attachments.map((attachment, index) => (
                              <AttachmentView
                                key={attachment.file_id ?? index}
                                attachment={attachment}
                                endpoint={endpoint}
                              />
                            ))}
                          </div>
                        )}
                        {turn && m?.role !== "user" && (() => {
                          const live = Object.entries(turn.decisions).filter(([, decision]) => decision.tool!=="ferry_ask_user"&&liveDecision(turn, decision));
                          const ended = Object.entries(turn.decisions).filter(([, decision]) => decision.tool!=="ferry_ask_user"&&endedDecision(turn, decision));
                          const questions = Object.entries(turn.decisions).filter(([, decision]) => decision.tool==="ferry_ask_user");
                          const owner = turn.owner_id === data.viewer_id;
                          const ownerName = data.members.find((member) => member.id === turn.owner_id)?.name || t("member");
                          const turnActivity = turnEvents.get(turn.id) ?? NO_EVENTS;
                          const hasDetails = !!turnActivity.length || !!items.size || !!turn.usage || Object.values(turn.decisions).some((decision) => decision.decision !== null || turn.state !== "running");
                          const counts = timelineSummary(turnActivity, turn.decisions, turn);
                          return (
                            <>
                              {questions.map(([request,decision])=><QuestionCard key={request} request={request} decision={decision} canAnswer={owner&&permissions.has("conversations.run")} busy={busy} executing={active(turn.state)} onAnswer={async response=>{
                                setBusy(true);try {await api(endpoint+`/turns/${turn.id}/question-answers`,{method:"POST",body:JSON.stringify({request_id:request,...response})});decisionRevision.current++;setData(await fetchConversation(endpoint));}finally{setBusy(false);}
                              }}/>) }
                              {live.map(([request, decision]) => (
                                <PendingDecisionCard
                                  key={request}
                                  request={request}
                                  decision={decision}
                                  canDecide={runnable && owner}
                                  ownerName={ownerName}
                                  busy={busy}
                                  onDecide={(allow) => void act(`/turns/${turn.id}/decisions`, { request_id: request, allow }, () => updateDecision(turn.id, request, { decision: allow }))}
                                />
                              ))}
                              {ended.map(([request, decision]) => (
                                <EndedDecisionCard
                                  key={request}
                                  request={request}
                                  decision={decision}
                                  canAct={permissions.has("conversations.run") && owner}
                                  busy={busy}
                                  continueDisabled={!runnable || !!current}
                                  onContinue={(operation) => void resolveEnded(turn, request, operation)}
                                  onDismiss={async () => {
                                    await api(endpoint + `/turns/${turn.id}/decision-resolutions`, {
                                      method: "POST", body: JSON.stringify({ request_id: request, action: "dismissed" }),
                                    });
                                    const revision = ++decisionRevision.current;
                                    updateDecision(turn.id, request, { resolved: "dismissed" });
                                    const latest = await fetchConversation(endpoint);
                                    if (revision === decisionRevision.current) setData(latest);
                                  }}
                                />
                              ))}
                              {hasDetails && (
                                <ExecutionDetailsPanel summary={counts ? `${t("executionDetails")} · ${counts}` : t("executionDetails")}>
                                  {() => (
                                    <ExecutionTimeline
                                    key={data.viewer_id}
                                    viewerId={data.viewer_id}
                                      events={turnActivity}
                                      decisions={turn.decisions}
                                      turnState={turn.state}
                                      usage={turn.usage}
                                      stateLabel={t(turn.state)}
                                      deciderName={ownerName}
                                    deciders={data.members}
                                    />
                                  )}
                                </ExecutionDetailsPanel>
                              )}
                            </>
                          );
                        })()}
                        {turn && m?.role !== "user" && (
                          <>
                            {(active(turn.state) || turn.state === "held") && (
                              <p
                                role="status"
                                className="flex items-center gap-2 text-xs text-muted-foreground"
                              >
                                <span className="size-1.5 animate-pulse rounded-full bg-current" />
                                {t(turn.state)}
                              </p>
                            )}
                            {(["failed", "interrupted", "timed_out", "cancelled"].includes(turn.state) || turn.error) && (
                              <div data-turn-failure={turn.id} className="space-y-1.5">
                                <p role="status" className="break-words text-sm text-destructive">
                                  {turn.error || t(turn.state)}
                                </p>
                                {/* A conversation in a chosen directory does not move by itself; the user may. */}
                                {turn.error_key === "conversations.hostUnavailable" && permissions.has("conversations.run") && turn.id === (data.kind === "group" ? data.turns.filter(item => item.agent_id === turn.agent_id).at(-1)?.id : lastTurn?.id) && (
                                  <Button type="button" variant="outline" size="sm" className="min-h-11" disabled={busy || data.turns.some(item => active(item.state))} onClick={() => void act("/host-releases", data.kind === "group" ? { agent_id: turn.agent_id } : {})}>
                                    {t("useAnotherHost")}
                                  </Button>
                                )}
                                {turn.purpose === "introduction" && <>
                                  {turn.introduction_retry_warning && <p className="break-words text-xs text-muted-foreground">{turn.introduction_retry_warning}</p>}
                                  {turn.can_retry_introduction && runnable ? <Button type="button" variant="outline" size="sm" className="min-h-11" disabled={busy || !!current} onClick={() => void retryIntroduction(turn)}><RotateCcw className="size-3.5" />{t("retryIntroduction")}</Button> : turn.introduction_retry_reason && <p className="break-words text-xs text-muted-foreground">{turn.introduction_retry_reason}</p>}
                                </>}
                                {turn.purpose !== "introduction" && data.kind === "private" && runnable && retryable && turn.id === lastTurn?.id && (
                                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                                    <Button type="button" variant="ghost" size="sm" className="min-h-11 px-2 text-foreground" disabled={busy || !!current} onClick={() => void send(turn)}>
                                      <RotateCcw className="size-3.5" />{t("retryLastTurn")}
                                    </Button>
                                    <span className="text-xs text-muted-foreground">{t("retryEffectsHint")}</span>
                                  </div>
                                )}
                              </div>
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
                            requestAnimationFrame(() =>
                              document
                                .querySelector<HTMLTextAreaElement>("#conversation-message")
                                ?.focus({ preventScroll: true }),
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
      {data.state === "archived" && (
        <p role="status" className="px-3 text-sm text-muted-foreground sm:px-6">{t("archived")}</p>
      )}
      {data.kind === "group" && data.turns.some(turn => turn.state === "held") && (
        <HeldRelays endpoint={endpoint} onChanged={async () => setData(await fetchConversation(endpoint))} />
      )}
      <div className="shrink-0 px-3 pb-3 pt-2 sm:px-6 sm:pb-5">
        {error && (
          <InlineAlert className="mb-2" onDismiss={() => setError("")}>
            {error}
          </InlineAlert>
        )}
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
          reply={thread ? (() => {
            const message = data.messages.find((m) => m.id === thread);
            return {
              name: message?.sender_name || data.agents?.find((a) => a.id === message?.agent_id)?.name || t("agent"),
              text: message?.retracted_at || !message ? t("retracted") : message.prompt || message.attachments?.map((a) => a.name).join(", ") || t("attach"),
            };
          })() : undefined}
          onCancelReply={() => setThread("")}
          onSend={async () => { await send(); }}
          queue={queue && data.kind !== "group" ? {
            state: queue.state,
            onCancel: () => setQueue(undefined),
            onSendNow: () => { setQueue(undefined); void send(); },
          } : undefined}
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
              <InlineAlert>{actionError}</InlineAlert>
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
              <InlineAlert>{actionError}</InlineAlert>
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

/** The model a private conversation runs with; an Agent without a model is called out. */
function ModelLabel({ model, hasModel }: { model: string; hasModel?: boolean | undefined }) {
  if (hasModel === false) return <span className="text-amber-700 dark:text-amber-400">{t("noModel")}</span>;
  return <>{model || t("nativeModel")}</>;
}
