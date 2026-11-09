import { FieldCaption } from "../../shared/forms/FieldHelp";
import { Dialog } from "radix-ui";
import {
  ArrowDownWideNarrow,
  ArrowRight,
  Braces,
  Clock,
  Filter,
  GitBranch,
  Layers,
  ListFilter,
  Merge,
  Plus,
  Search,
  Square,
  Text,
  Workflow,
  X,
  Zap,
  File,
  Image,
  Music,
  Video,
  Plug,
  ChevronLeft,
  ArrowUp,
  Terminal,
  Eye,
} from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import { workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { manifest, type GraphNode, type Port } from "./graph";
import {
  integrationManifests,
  integrationManifest,
  integrationOperationTitle,
  integrationTitle,
  integrationText,
} from "./integrations";

const builtinCatalog = [
  { type: "host.command", key: "hostCommand", group: "build", icon: Terminal },
  { type: "host.preview", key: "hostPreview", group: "build", icon: Eye },
  { type: "ai.llm.chat", key: "aiChat", group: "ai", icon: Braces },
  { type: "ai.image.generate", key: "aiImage", group: "ai", icon: Image },
  { type: "ai.audio.speech", key: "aiSpeech", group: "ai", icon: Music },
  { type: "ai.audio.transcribe", key: "aiTranscribe", group: "ai", icon: Text },
  { type: "file.input", key: "fileInput", group: "media", icon: File },
  { type: "image.input", key: "imageInput", group: "media", icon: Image },
  { type: "audio.input", key: "audioInput", group: "media", icon: Music },
  { type: "video.input", key: "videoInput", group: "media", icon: Video },
  { type: "file.encode", key: "fileEncode", group: "media", icon: File },
  { type: "file.decode", key: "fileDecode", group: "media", icon: File },
  { type: "core.trigger.manual", key: "manual", group: "triggers", icon: Zap },
  { type: "data.set", key: "set", group: "data", icon: Braces },
  { type: "core.if", key: "if", group: "flow", icon: GitBranch },
  { type: "core.switch", key: "switch", group: "flow", icon: GitBranch },
  { type: "core.filter", key: "filter", group: "data", icon: Filter },
  { type: "core.merge", key: "merge", group: "flow", icon: Merge },
  { type: "core.wait", key: "wait", group: "flow", icon: Clock },
  { type: "core.no_op", key: "no_op", group: "flow", icon: ArrowRight },
  { type: "core.limit", key: "limit", group: "data", icon: ListFilter },
  { type: "core.sort", key: "sort", group: "data", icon: ArrowDownWideNarrow },
  { type: "core.split", key: "split", group: "data", icon: Layers },
  { type: "data.rename", key: "rename", group: "data", icon: Braces },
  { type: "data.text_split", key: "text_split", group: "data", icon: Text },
  { type: "core.stop", key: "stop", group: "flow", icon: Square },
] as const;
const catalog = () => [
  ...builtinCatalog.map((node) => ({
    ...node,
    application: "",
    resource: "",
    title: () => t(node.key),
    hint: () => t(`nodeHint.${node.key}`),
    keywords: manifest(node.type)?.search_aliases?.join(" ") ?? "",
  })),
  ...[...integrationManifests].sort((left, right) => Number(right.git?.operation === "clone") - Number(left.git?.operation === "clone")).map((node) => ({
    type: node.type,
    group: node.git && !node.git.provider ? "build" : "integrations",
    icon: Plug,
    application: node.ui.application,
    resource: node.ui.resource_key,
    title: () => integrationOperationTitle(node),
    hint: () => node.ui.application_name,
    keywords: `${node.ui.application} ${node.ui.aliases.join(" ")} ${node.ui.resource_key} ${node.ui.operation_key}`,
  })),
];
const groups = [
  "triggers",
  "flow",
  "data",
  "media",
  "ai",
  "build",
  "integrations",
] as const;
export const nodeTitle = (type: string) => {
  const integration = integrationManifest(type);
  if (integration) return integrationTitle(integration);
  const node = builtinCatalog.find((node) => node.type === type);
  return node ? t(node.key) : type;
};

export function outputPortLabel(node: GraphNode, port: Pick<Port, "port" | "index">): string {
  if (node.type === "core.if" && (port.port === "true" || port.port === "false"))
    return t(port.port === "true" ? "branchTrue" : "branchFalse");
  if (node.type === "host.preview" && (port.port === "approved" || port.port === "rejected"))
    return t(port.port === "approved" ? "branchApproved" : "branchRejected");
  if (node.type === "core.switch") {
    if (port.port === "fallback") return t("branchFallback");
    if (port.port === "case") {
      const branch = (node.params.cases as {key: string}[] | undefined)?.[port.index ?? 0];
      return branch?.key || `${t("branch")} ${(port.index ?? 0) + 1}`;
    }
  }
  return `${port.port}${port.index === undefined ? "" : ` ${port.index + 1}`}`;
}

/** Add and inline insertion share one searchable, keyboard accessible picker. */
export function WorkflowNodePicker({
  inserting,
  accepts,
  select,
  close,
  connection,
}: {
  inserting: boolean;
  accepts: (type: string) => boolean;
  select: (type: string) => void;
  close: () => void;
  connection?:
    | {
        ports: Port[];
        value: string;
        change: (value: string) => void;
        label?: (port: Port) => string;
      }
    | undefined;
}) {
  const [search, setSearch] = useState("");
  const [group, setGroup] = useState("all");
  const [application, setApplication] = useState("");
  const [resource, setResource] = useState("");
  const scroller = useRef<HTMLDivElement>(null);
  const positions = useRef(new Map<string, number>());
  const [scrollTop, setScrollTop] = useState(0);
  const searching = !!search.trim();
  const route = searching ? `search:${search.trim()}` : JSON.stringify([group, application, resource]);
  useLayoutEffect(() => {
    const top = positions.current.get(route) ?? 0;
    if (scroller.current) scroller.current.scrollTop = top;
    setScrollTop(top);
  }, [route]);
  const available = catalog().filter(
    (node) =>
      accepts(node.type) &&
      (searching || group === "all" || node.group === group) &&
      (searching || !application || node.application === application) &&
      (searching || !resource || node.resource === resource) &&
      `${node.title()} ${node.hint()} ${node.type} ${node.keywords}`
        .toLocaleLowerCase()
        .includes(search.trim().toLocaleLowerCase()),
  );
  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/20" />
        <Dialog.Content className="fixed inset-y-0 right-0 z-50 flex min-h-0 w-full max-w-md flex-col overflow-hidden border-l bg-background shadow-xl">
          <div className="shrink-0 border-b p-5">
            <div className="flex items-center justify-between gap-3">
              <Dialog.Title className="flex items-center gap-2 text-lg font-semibold">
                <Workflow className="size-5" />
                {t(inserting ? "insert" : "add")}
              </Dialog.Title>
              <Dialog.Close asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={t("close")}
                >
                  <X />
                </Button>
              </Dialog.Close>
            </div>
            <Dialog.Description className="mt-2 text-sm leading-6 text-muted-foreground">
              {t(
                connection?.ports.length === 0
                  ? "terminalAddHelp"
                  : inserting
                    ? "insertHelp"
                    : "addHelp",
              )}
            </Dialog.Description>
            <div className="relative mt-4">
              <Search className="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground" />
              <Input
                autoFocus
                aria-label={t("search")}
                placeholder={t("search")}
                className="h-10 pl-9"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
              />
            </div>
            {connection && connection.ports.length > 1 && (
              <label className="mt-4 block space-y-2 text-sm">
                <span><FieldCaption label={<>{t("continueFrom")}</>} /></span>
                <select
                  aria-label={t("continueFrom")}
                  className="h-10 w-full rounded-md border bg-background px-3"
                  value={connection.value}
                  onChange={(event) => connection.change(event.target.value)}
                >
                  {connection.ports.map((port) => (
                    <option
                      key={`${port.port}:${port.index ?? 0}`}
                      value={`${port.port}:${port.index ?? 0}`}
                    >
                      {connection.label?.(port) ?? `${port.port}${port.index === undefined ? "" : ` ${port.index + 1}`}`}
                    </option>
                  ))}
                </select>
              </label>
            )}
            <div
              role="group"
              aria-label={t("categories")}
              className="mt-3 flex flex-wrap gap-1"
            >
              {(["all", ...groups] as const).map((key) => (
                <Button
                  key={key}
                  type="button"
                  variant={group === key ? "secondary" : "ghost"}
                  size="sm"
                  aria-pressed={group === key}
                  onClick={() => {
                    setGroup(key);
                    setApplication("");
                    setResource("");
                  }}
                >
                  {t(`category.${key}`)}
                </Button>
              ))}
            </div>
          </div>
          {!searching && application && <nav aria-label={t("picker.breadcrumb")} className="flex flex-wrap items-center gap-1 border-b px-3 py-2 text-sm"><Button type="button" variant="ghost" size="sm" onClick={() => {setApplication("");setResource("");}}>{t("category.integrations")}</Button><span>/</span><Button type="button" variant="ghost" size="sm" onClick={() => setResource("")}>{integrationManifests.find(node => node.ui.application === application)?.ui.application_name.replace(/^literal:/, "") ?? application}</Button>{resource && <><span>/</span><span aria-current="page">{integrationText(resource)}</span></>}</nav>}
          <div ref={scroller} aria-label={t("picker.results")} className="min-h-0 flex-1 overflow-y-auto p-3" onScroll={event => { const top = event.currentTarget.scrollTop; positions.current.set(route, top); setScrollTop(top); }}>
            {!searching && application && (
              <Button
                type="button"
                variant="ghost"
                className="mb-3"
                onClick={() => { if (resource) setResource(""); else setApplication(""); }}
              >
                <ChevronLeft />
                {t("integration.back")}
              </Button>
            )}
            {groups.map((group) => {
              const nodes = available.filter((node) => node.group === group);
              return (
                nodes.length > 0 && (
                  <section
                    key={group}
                    aria-label={t(`category.${group}`)}
                    className="mb-5"
                  >
                    <h3 className="px-3 pb-2 pt-2 text-xs font-semibold text-muted-foreground">
                      {t(`category.${group}`)}
                    </h3>
                    {group === "integrations" && !application && !searching
                      ? [...new Set(nodes.map((node) => node.application))].map(
                          (app) => {
                            const operations = nodes.filter(
                              (node) => node.application === app,
                            );
                            const name = operations[0]!.hint();
                            return (
                              <button
                                key={app}
                                type="button"
                                aria-label={name}
                                className="flex min-h-16 w-full items-center gap-3 rounded-lg px-3 py-3 text-left hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                                onClick={() => { setApplication(app); setResource(""); }}
                              >
                                <span className="grid size-10 shrink-0 place-items-center rounded-lg border bg-muted/40">
                                  <Plug className="size-5" />
                                </span>
                                <span className="flex-1">
                                  <span className="block text-sm font-medium">
                                    {name}
                                  </span>
                                  <span className="mt-1 block text-xs text-muted-foreground">
                                    {operations.length}{" "}
                                    {t("integration.operationCount")}
                                  </span>
                                </span>
                                <ArrowRight className="size-4" />
                              </button>
                            );
                          },
                        )
                      : group === "integrations" && application && !resource && !searching
                        ? [...new Set(nodes.map(node => node.resource))].map(key => <button key={key} type="button" aria-label={integrationText(key)} className="flex min-h-16 w-full items-center justify-between gap-3 rounded-lg px-3 py-3 text-left hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring" onClick={() => setResource(key)}><span><span className="block text-sm font-medium">{integrationText(key)}</span><span className="text-xs text-muted-foreground">{nodes.filter(node => node.resource === key).length} {t("integration.operationCount")}</span></span><ArrowRight className="size-4" /></button>)
                      : nodes.map((node) => (
                          <button
                            key={node.type}
                            type="button"
                            aria-label={
                              node.application
                                ? `${node.hint()} · ${node.title()}`
                                : node.title()
                            }
                            className="group flex min-h-16 w-full items-center gap-3 rounded-lg px-3 py-3 text-left hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                            onClick={() => select(node.type)}
                          >
                            <span className="grid size-10 shrink-0 place-items-center rounded-lg border bg-muted/40">
                              <node.icon className="size-5" />
                            </span>
                            <span className="min-w-0 flex-1">
                              <span className="block text-sm font-medium">
                                {node.title()}
                              </span>
                              <span className="mt-1 block text-xs leading-5 text-muted-foreground">
                                {node.hint()}
                              </span>
                            </span>
                            <Plus className="size-4 shrink-0 text-muted-foreground group-hover:text-foreground" />
                          </button>
                        ))}
                  </section>
                )
              );
            })}
            {!available.length && (
              <div role="status" className="px-4 py-12 text-center">
                <Search className="mx-auto mb-3 size-6 text-muted-foreground" />
                <p className="text-sm font-medium">{t("noNodes")}</p>
                <p className="mt-2 text-xs leading-5 text-muted-foreground">
                  {t("searchHelp")}
                </p>
                <Button
                  type="button"
                  variant="outline"
                  className="mt-4"
                  onClick={() => {
                    setSearch("");
                  }}
                >
                  {t("clearSearch")}
                </Button>
              </div>
            )}
          </div>
          {scrollTop > 200 && <div className="flex justify-end border-t p-2"><Button type="button" variant="ghost" size="sm" onClick={() => scroller.current?.scrollTo({top:0,behavior:"smooth"})}><ArrowUp className="size-4" />{t("picker.backToTop")}</Button></div>}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
