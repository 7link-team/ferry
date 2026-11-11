import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "cn";
import {
  Bot,
  Menu,
  MessageSquare,
  Pencil,
  Plus,
  Search,
  Users,
  X,
} from "lucide-react";
import { Dialog } from "radix-ui";
import { href, navigate, useRoute } from "../router";
import { ResourceForm } from "../resources/ResourceForm";
import { errorText } from "../../shared/api/api";
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
} from "../../features/conversations/Conversation";
import { conversationText as t } from "../../features/conversations/resource-config";

interface Summary {
  id: string;
  title: string;
  agent_name: string;
  state: string;
  kind?: string;
  unread?: number;
  mentions?: number;
}
export function ConversationsWorkspace({
  workspace,
  permissions,
  form,
}: {
  workspace: string;
  permissions: Set<string>;
  form: FormConfig<AnyRow> | undefined;
}) {
  useCommonTranslation();
  const route = useRoute();
  const [rows, setRows] = useState<Summary[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState("");
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [pages, setPages] = useState(1);
  const [total, setTotal] = useState(0);
  const [mobile, setMobile] = useState(false);
  const [revision, setRevision] = useState(0);
  const readable = permissions.has("conversations.read");
  const editing = route.mode === "new" || route.mode === "edit";
  const selected = route.mode !== "new" ? route.id : undefined;
  const drafts = useRef(new Map<string, ConversationDraft>());
  const saveDraft = useCallback(
    (draft: ConversationDraft) => {
      if (selected) drafts.current.set(selected, draft);
    },
    [selected],
  );
  useEffect(() => {
    const timer = setTimeout(() => {
      setQuery(search);
      setPages(1);
    }, 200);
    return () => clearTimeout(timer);
  }, [search]);
  useEffect(() => {
    if (!readable) {
      setRows([]);
      return;
    }
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
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
        setRows([
          ...new Map(
            results
              .flatMap((result) => result.items)
              .map((row) => [row.id, row]),
          ).values(),
        ]);
        setTotal(results[0]?.total ?? 0);
        setLoaded(true);
        setError("");
      } catch (cause) {
        if (controller.signal.aborted) return;
        setRows([]);
        setError(errorText(cause));
        setLoaded(true);
      } finally {
        if (!controller.signal.aborted)
          timer = setTimeout(() => void refresh(), 3000);
      }
    }
    void refresh();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [workspace, readable, query, pages, revision]);
  useEffect(() => {
    if (route.mode === "list" && loaded && !query && !error && rows[0])
      navigate(
        {
          workspace,
          group: "conversations",
          item: "conversations",
          mode: "view",
          id: rows[0].id,
        },
        {},
        { replace: true },
      );
  }, [workspace, route.mode, loaded, rows, query, error]);
  useEffect(() => {
    setMobile(false);
  }, [route.id, route.mode]);
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
        className="min-h-0 flex-1 space-y-1 overflow-y-auto p-2"
      >
        {!loaded && <LoadingSkeleton variant="page" />}
        {error && (
          <p role="alert" className="p-3 text-sm text-destructive">
            {error}
          </p>
        )}
        {loaded && !error && !rows.length && (
          <p className="p-3 text-sm text-muted-foreground">
            {t(query ? "noSearchResults" : "noConversations")}
          </p>
        )}
        {rows.map((row) => (
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
              <span className="block truncate font-medium">{row.title}</span>
              <span className="mt-1 flex min-w-0 gap-2 text-xs opacity-70">
                <span className="truncate">
                  {row.kind === "group" ? t("group") : row.agent_name}
                </span>
                {row.state !== "active" && (
                  <span className="shrink-0">{t(row.state)}</span>
                )}
              </span>
            </span>
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
        ))}
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
  const mobileButton = (
    <Button
      className="sm:hidden"
      variant="ghost"
      size="icon"
      aria-label={t("conversationList")}
      onClick={() => setMobile(true)}
    >
      <Menu />
    </Button>
  );
  return (
    <div className="grid min-h-0 min-w-0 grid-cols-1 sm:grid-cols-[260px_minmax(0,1fr)]">
      <aside className="hidden min-h-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground sm:flex">
        {sidebar}
      </aside>
      <main className="relative flex min-h-0 min-w-0 flex-col">
        <div className="absolute left-2 top-3 z-10 sm:hidden">
          {mobileButton}
        </div>
        {!readable ? (
          <p role="alert">{commonText("shell.forbidden")}</p>
        ) : selected ? (
          <Conversation
            key={`${workspace}/${selected}`}
            workspace={workspace}
            id={selected}
            permissions={permissions}
            initialDraft={drafts.current.get(selected)}
            onDraftChange={saveDraft}
            onOpenConversation={id => { setRevision(value => value + 1); go(id, "view"); }}
            onDeleted={() => {
              drafts.current.delete(selected);
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
                </Button>
              )
            }
          />
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
      <Dialog.Root open={mobile} onOpenChange={setMobile}>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-40 bg-black/40 sm:hidden" />
          <Dialog.Content
            aria-describedby={undefined}
            className="fixed inset-y-0 left-0 z-50 flex w-[min(20rem,85vw)] flex-col border-r bg-sidebar text-sidebar-foreground shadow-lg sm:hidden"
          >
            <div className="flex items-center justify-between px-4 pt-3">
              <Dialog.Title className="font-medium">
                {t("conversationList")}
              </Dialog.Title>
              <Dialog.Close asChild>
                <Button
                  size="icon"
                  variant="ghost"
                  aria-label={commonText("common.close")}
                >
                  <X />
                </Button>
              </Dialog.Close>
            </div>
            {sidebar}
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
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
