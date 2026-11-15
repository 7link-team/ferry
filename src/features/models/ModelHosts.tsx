import { useEffect, useState } from "react";
import { ArrowDown, ArrowUp, X } from "lucide-react";
import { errorText } from "../../shared/api/api";
import { listResource } from "../../shared/api/resource";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";

const t = (key: string, values?: Record<string, string>) => commonText(`models:${key}`, values);

interface RunnerHost {
  id: string;
  name: string;
  state: string;
}

/**
 * The Runner hosts a CLI model runs on, first preferred. Empty means any
 * online Runner that has the CLI ready; the order is the failover order.
 */
export function ModelHosts({
  workspace,
  value,
  readOnly,
  onChange,
}: {
  workspace: string;
  value: string[];
  readOnly: boolean;
  onChange: (hosts: string[]) => void;
}) {
  const [hosts, setHosts] = useState<RunnerHost[]>([]);
  const [error, setError] = useState("");
  useEffect(() => {
    const controller = new AbortController();
    void (async () => {
      try {
        const items: RunnerHost[] = [];
        for (let page = 1; ; page++) {
          const r = await listResource<RunnerHost>(workspace, "hosts", { connection: "runner", page, page_size: 200 }, controller.signal);
          items.push(...r.items);
          if (page * r.page_size >= r.total || !r.items.length) break;
        }
        if (!controller.signal.aborted) setHosts(items);
      } catch (e) {
        if (!controller.signal.aborted) setError(errorText(e));
      }
    })();
    return () => controller.abort();
  }, [workspace]);
  const name = (id: string) => hosts.find((host) => host.id === id)?.name ?? id;
  const offline = (id: string) => hosts.find((host) => host.id === id)?.state === "offline";
  const move = (index: number, by: number) => {
    const next = [...value];
    const [item] = next.splice(index, 1);
    next.splice(index + by, 0, item!);
    onChange(next);
  };
  const available = hosts.filter((host) => !value.includes(host.id));
  return (
    <AutoField label={t("executionHosts")} help={t("executionHostsHelp")}>
      <div className="space-y-2">
        {value.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("executionHostsAuto")}</p>
        ) : (
          <ol aria-label={t("executionHosts")} className="space-y-1">
            {value.map((id, index) => (
              <li key={id} className="flex min-h-9 items-center gap-2 rounded-md border px-2 py-1 text-sm">
                <span className="w-5 shrink-0 text-right text-muted-foreground">{index + 1}.</span>
                <span className="min-w-0 flex-1 truncate">{name(id)}</span>
                {offline(id) && <span className="shrink-0 text-xs text-muted-foreground">{t("executionHostsOffline")}</span>}
                {!readOnly && (
                  <span className="flex shrink-0 gap-1">
                    <Button type="button" variant="ghost" size="icon" className="size-7" disabled={index === 0} aria-label={t("executionHostsUp", { host: name(id) })} onClick={() => move(index, -1)}><ArrowUp className="size-4" /></Button>
                    <Button type="button" variant="ghost" size="icon" className="size-7" disabled={index === value.length - 1} aria-label={t("executionHostsDown", { host: name(id) })} onClick={() => move(index, 1)}><ArrowDown className="size-4" /></Button>
                    <Button type="button" variant="ghost" size="icon" className="size-7" aria-label={t("executionHostsRemove", { host: name(id) })} onClick={() => onChange(value.filter((host) => host !== id))}><X className="size-4" /></Button>
                  </span>
                )}
              </li>
            ))}
          </ol>
        )}
        {!readOnly && available.length > 0 && (
          <select
            aria-label={t("executionHostsAdd")}
            className="h-9 w-full rounded-md border bg-background px-2 text-sm"
            value=""
            onChange={(event) => event.target.value && onChange([...value, event.target.value])}
          >
            <option value="">{t("executionHostsAdd")}</option>
            {available.map((host) => (
              <option key={host.id} value={host.id}>
                {host.name}{host.state === "offline" ? ` (${t("executionHostsOffline")})` : ""}
              </option>
            ))}
          </select>
        )}
        {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      </div>
    </AutoField>
  );
}
