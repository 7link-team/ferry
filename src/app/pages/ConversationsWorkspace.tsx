import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "cn";
import {
  ArrowLeft,
  Bot,
  MessageSquare,
  Pencil,
  Plus,
  Search,
  Users,
} from "lucide-react";
import { Dialog } from "radix-ui";
import { href, navigate, useRoute } from "../router";
import { ResourceForm } from "../resources/ResourceForm";
import { ApiError, errorText } from "../../shared/api/api";
import { canResumeSession } from "../../shared/api/session";
import { watchPageResume } from "../../shared/api/use-auto-refresh";
import { listResource } from "../../shared/api/resource";
import { commonText, useCommonTranslation } from "../../shared/i18n/i18n";
import type { AnyRow } from "../../shared/resources/config";
import type { FormConfig } from "../../shared/resources/types";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import {
  Conversation,
  type ConversationDraft,
  type ConversationSnapshot,
} from "../../features/conversations/Conversation";
import { conversationText as t } from "../../features/conversations/resource-config";
import { InlineAlert } from "../../shared/ui/inline-alert";
import { usePhone } from "../../shared/ui/use-phone";

interface Summary {
  id: string;
  title: string;
  agent_name: string;
  state: string;
  kind?: string;
  unread?: number;
  mentions?: number;
  /** Approvals waiting for the viewer, including ones left when a reply ended. */
  pending?: number;
  /** The latest message, as a one-line preview. */
  last_message?: { text: string; sender_name: string; role: string; created_at: number } | null;
}
/** Messaging-app list time: clock time today, otherwise the date. */
function previewTime(at: number) {
  const date = new Date(at), now = new Date();
  return date.toDateString() === now.toDateString()
    ? date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString(undefined, { month: "numeric", day: "numeric" });
}
/** Owned by the authenticated shell; never persisted to browser storage. */
export interface ConversationsSession {
  list?: { rows: Summary[]; total: number; query: string; pages: number } | undefined;
  selected?: string | undefined;
  snapshot?: ConversationSnapshot | undefined;
  drafts: Map<string, ConversationDraft>;
}
export function ConversationsWorkspace({
  workspace,
  permissions,
  form,
  session: storedSession,
  appActions,
}: {
  workspace: string;
  permissions: Set<string>;
  form: FormConfig<AnyRow> | undefined;
  session?: ConversationsSession;
  appActions?: ReactNode;
}) {
  useCommonTranslation();
  const route = useRoute();
  const [localSession] = useState<ConversationsSession>(() => ({ drafts: new Map() }));
  const session = storedSession ?? localSession;
  const [rows, setRows] = useState<Summary[]>(session.list?.rows ?? []);
  const [loaded, setLoaded] = useState(!!session.list);
  const [error, setError] = useState("");
  const [denied, setDenied] = useState(false);
  const [search, setSearch] = useState(session.list?.query ?? "");
  const [query, setQuery] = useState(session.list?.query ?? "");
  const [pages, setPages] = useState(session.list?.pages ?? 1);
  const [total, setTotal] = useState(session.list?.total ?? 0);
  // Phones work like messaging apps: the list first, a conversation as its own page.
  const phone = usePhone();
  const [revision, setRevision] = useState(0);
  const [activePending, setActivePending] = useState<{ id: string; count: number }>();
  const updatePending = useCallback((id: string, count: number) => {
    setActivePending(previous => previous?.id === id && previous.count === count ? previous : { id, count });
  }, []);
  const readable = permissions.has("conversations.read");
  const editing = route.mode === "new" || route.mode === "edit";
  const selected = route.mode === "new" ? undefined : route.id ?? (route.mode === "list" && loaded
    && !phone ? session.selected ?? (!query ? rows[0]?.id : undefined) : undefined);
  const drafts = useRef(session.drafts);
  const saveSnapshot = useCallback((snapshot: ConversationSnapshot | undefined) => {
    session.snapshot = snapshot;
  }, [session]);
  useEffect(() => {
    if (selected) session.selected = selected;
  }, [selected, session]);
  const saveDraft = useCallback(
    (draft: ConversationDraft) => {
      if (selected) drafts.current.set(selected, draft);
    },
    [selected],
  );
  useEffect(() => {
    const timer = setTimeout(() => {
      setQuery(search);
      if (search !== query) setPages(1);
    }, 200);
    return () => clearTimeout(timer);
  }, [search]);
  useEffect(() => {
    if (!readable) {
      setRows([]);
      session.list = undefined;
      session.snapshot = undefined;
      session.selected = undefined;
      session.drafts.clear();
      return;
    }
    let controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function refresh(controller: AbortController) {
      if (document.hidden) return;
      try {
        const results = await Promise.all(
          Array.from({ length: pages }, (_, index) =>
            listResource<Summary>(
              workspace,
              "conversations",
              { page: index + 1, page_size: 50, q: query, sort: "-updated_at" },
              controller.signal,
            ),
          ),
        );
        if (controller.signal.aborted) return;
        const rows = [
          ...new Map(
            results
              .flatMap((result) => result.items)
              .map((row) => [row.id, row]),
          ).values(),
        ];
        const total = results[0]?.total ?? 0;
        session.list = { rows, total, query, pages };
        setRows(rows);
        setTotal(total);
        setLoaded(true);
        setDenied(false);
        setError("");
      } catch (cause) {
        if (controller.signal.aborted) return;
        if (cause instanceof ApiError && [401, 403].includes(cause.status) &&
            !(cause.status === 401 && canResumeSession())) {
          setDenied(true);
          setRows([]);
          session.list = undefined;
          session.snapshot = undefined;
          session.selected = undefined;
          session.drafts.clear();
        }
        setError(errorText(cause));
        setLoaded(true);
      } finally {
        if (!controller.signal.aborted && !document.hidden)
          timer = setTimeout(() => void refresh(controller), 3000);
      }
    }
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
  }, [workspace, readable, query, pages, revision, session]);
  useEffect(() => {
    if (route.mode === "list" && loaded && !error && selected)
      navigate(
        {
          workspace,
          group: "conversations",
          item: "conversations",
          mode: "view",
          id: selected,
        },
        {},
        { replace: true },
      );
  }, [workspace, route.mode, loaded, rows, query, error, selected]);
  const go = (id: string | undefined, mode: "new" | "view" | "edit" | "list") =>
    navigate(
      {
        workspace,
        group: "conversations",
        item: "conversations",
        mode,
        ...(id ? { id } : {}),
      },
      {},
    );
  const closeForm = () => go(selected, selected ? "view" : "list");
  const sidebar = (
    <>
      <div className="space-y-3 border-b border-sidebar-border p-4">
        <div className="flex items-center justify-between gap-2">
          <h1 className="text-sm font-semibold">{t("conversationList")}</h1>
          {permissions.has("conversations.create") && (
            <Button
              size="icon"
              variant="ghost"
              aria-label={t("newConversation")}
              title={t("newConversation")}
              onClick={() => go(undefined, "new")}
            >
              <Plus />
            </Button>
          )}
        </div>
        <div className="relative">
          <Search
            aria-hidden
            className="pointer-events-none absolute left-2.5 top-2.5 size-4 text-muted-foreground"
          />
          <Input
            aria-label={t("searchConversations")}
            placeholder={t("searchConversations")}
            className="pl-9"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </div>
      </div>
      <nav
        aria-label={t("conversationList")}
        className="min-h-0 flex-1 basis-0 space-y-1 overflow-y-auto p-2"
      >
        {!loaded && <LoadingSkeleton variant="page" />}
        {error && (
          <InlineAlert className="m-1">{error}</InlineAlert>
        )}
        {loaded && !error && !rows.length && (
          <p className="p-3 text-sm text-muted-foreground">
            {t(query ? "noSearchResults" : "noConversations")}
          </p>
        )}
        {(readable && !denied ? rows : []).map((listed) => {
          // The selected conversation's detail is authoritative; a slow list poll
          // must not put back the badge that was just cleared by its owner.
          const row = activePending && activePending.id === selected && listed.id === selected ? { ...listed, pending: activePending.count } : listed;
          return (
          <a
            key={row.id}
            href={href(
              {
                workspace,
                group: "conversations",
                item: "conversations",
                mode: "view",
                id: row.id,
              },
              {},
            )}
            onClick={(event) => {
              if (
                event.button !== 0 ||
                event.metaKey ||
                event.ctrlKey ||
                event.shiftKey ||
                event.altKey
              )
                return;
              event.preventDefault();
              go(row.id, "view");
            }}
            aria-current={selected === row.id ? "page" : undefined}
            className={cn(
              "flex items-center gap-3 rounded-lg px-3 py-3 text-sm outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
              selected === row.id
                ? "sidebar-active-item bg-sidebar-primary text-sidebar-primary-foreground"
                : "hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
            )}
            title={row.title}
          >
            <span
              aria-hidden
              className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-current/10"
            >
              {row.kind === "group" ? (
                <Users className="size-5" />
              ) : (
                <Bot className="size-5" />
              )}
            </span>
            <span className="min-w-0 flex-1">
              <span className="flex min-w-0 items-baseline gap-2">
                <span className="min-w-0 flex-1 truncate font-medium">{row.title}</span>
                {row.last_message && <time data-conversation-time className="shrink-0 text-xs opacity-60">{previewTime(row.last_message.created_at)}</time>}
              </span>
              <span className="mt-1 flex min-w-0 gap-2 text-xs opacity-70">
                <span data-conversation-preview className="truncate">
                  {row.last_message
                    ? `${row.kind === "group" && row.last_message.sender_name ? `${row.last_message.sender_name}: ` : ""}${row.last_message.text}`
                    : row.kind === "group" ? t("group") : row.agent_name}
                </span>
                {row.state !== "active" && (
                  <span className="shrink-0">{t(row.state)}</span>
                )}
              </span>
            </span>
            {!!row.pending && (
              <span
                data-pending-badge
                className="shrink-0 rounded-full bg-amber-500 px-1.5 py-0.5 text-xs font-medium text-white"
                aria-label={`${t("pendingFilter")} ${row.pending}`}
                title={t("pendingFilter")}
              >
                {row.pending > 99 ? "99+" : row.pending}
              </span>
            )}
            {!!row.unread && (
              <span
                className="flex shrink-0 items-center gap-1 text-xs"
                aria-label={`${t("unread")} ${row.unread}`}
              >
                {!!row.mentions && <span>@</span>}
                <span className="rounded-full bg-sidebar-foreground px-1.5 py-0.5 text-sidebar">
                  {row.unread > 99 ? "99+" : row.unread}
                </span>
              </span>
            )}
          </a>
        ); })}
        {rows.length < total && (
          <Button
            className="w-full"
            variant="ghost"
            onClick={() => setPages((value) => value + 1)}
          >
            {t("loadMore")}
          </Button>
        )}
      </nav>
    </>
  );
  const backToList = (
    <Button
      className="size-11 md:hidden"
      variant="ghost"
      size="icon"
      aria-label={t("conversationList")}
      onClick={() => go(undefined, "list")}
    >
      <ArrowLeft />
    </Button>
  );
  return (
    <div className="grid min-h-0 min-w-0 grid-cols-1 md:grid-cols-[260px_minmax(0,1fr)] [&_.conversation-feed]:basis-0 [&_.conversation-feed]:overscroll-y-auto">
      <aside className="hidden min-h-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground md:flex">
        {sidebar}
      </aside>
      <main className="relative flex min-h-0 min-w-0 flex-col">
        {(!selected || !readable) && <div className="flex shrink-0 items-center gap-2 border-b px-2 py-1 pt-[max(.25rem,env(safe-area-inset-top))] md:hidden"><div className="ml-auto flex min-w-0 flex-wrap items-center gap-1">{appActions}</div></div>}
        {!readable || denied ? (
          <p role="alert">{error || commonText("shell.forbidden")}</p>
        ) : selected ? (
          <Conversation
            key={`${workspace}/${selected}`}
            workspace={workspace}
            id={selected}
            permissions={permissions}
            navigationAction={backToList}
            appActions={appActions}
            initialDraft={drafts.current.get(selected)}
            initialSnapshot={session.snapshot}
            onSnapshotChange={saveSnapshot}
            onDraftChange={saveDraft}
            onPendingChange={updatePending}
            onOpenConversation={id => { setRevision(value => value + 1); go(id, "view"); }}
            onDeleted={() => {
              drafts.current.delete(selected);
              session.snapshot = undefined;
              session.selected = undefined;
              if (session.list) session.list.rows = session.list.rows.filter(row => row.id !== selected);
              setRows((old) => old.filter((row) => row.id !== selected));
              setRevision((value) => value + 1);
              go(undefined, "list");
            }}
            headerActions={
              permissions.has("conversations.update") && (
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("editConversation")}
                  onClick={() => go(selected, "edit")}
                >
                  <Pencil />
                  <span data-conversation-action-label className="md:hidden">{t("editConversation")}</span>
                </Button>
              )
            }
          />
        ) : phone && route.mode !== "new" && rows.length > 0 ? (
          <div data-mobile-conversation-list className="flex min-h-0 flex-1 flex-col bg-sidebar text-sidebar-foreground">{sidebar}</div>
        ) : !loaded && route.mode !== "new" ? (
          <LoadingSkeleton variant="form" />
        ) : error && !rows.length && route.mode !== "new" ? (
          <InlineAlert>{error}</InlineAlert>
        ) : (
          <>
            <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 text-center">
              <MessageSquare
                className="size-8 text-muted-foreground"
                aria-hidden
              />
              <h1 className="text-xl font-medium">{t("startConversation")}</h1>
              <p className="max-w-sm text-sm text-muted-foreground">
                {t("empty")}
              </p>
              {permissions.has("conversations.create") && (
                <Button onClick={() => go(undefined, "new")}>
                  <Plus />
                  {t("newConversation")}
                </Button>
              )}
            </div>
          </>
        )}
      </main>
      <Dialog.Root
        open={editing}
        onOpenChange={(open) => {
          if (!open) closeForm();
        }}
      >
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-40 bg-black/40" />
          <Dialog.Content
            aria-describedby={undefined}
            className="dialog-shell fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-lg border bg-background p-5 shadow-lg"
          >
            <Dialog.Title className="mb-4 text-lg font-medium">
              {t(route.mode === "new" ? "newConversation" : "editConversation")}
            </Dialog.Title>
            {form && (
              <ResourceForm
                key={`${route.mode}/${route.id ?? "new"}`}
                config={form}
                workspace={workspace}
                permissions={permissions}
                createInPlace={route.mode === "new"}
                editInPlace={route.mode === "edit"}
                onCancel={closeForm}
                onSaved={(id) => {
                  setRevision((value) => value + 1);
                  go(id, id ? "view" : "list");
                }}
              />
            )}
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}
