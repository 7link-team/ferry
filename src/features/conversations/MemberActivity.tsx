import { useEffect, useMemo, useState } from "react";
import { i18n } from "../../shared/i18n/i18n";
import { conversationText as t } from "./resource-config";

export interface ActivityTurn {
  id: string;
  agent_id?: string | undefined;
  state: string;
  error?: string | undefined;
  created_at?: number | undefined;
  updated_at?: number | undefined;
  decisions: Record<string, { tool?: string; decision: boolean | null; sent: boolean }>;
}
export type ActivityItems = ReadonlyMap<string, ReadonlyMap<string, { name?: string | undefined; state?: string | undefined }>>;

const running = (state: string) => ["queued", "running", "stopping"].includes(state);

/** "1 min 5 s" / "42 s", localized units. */
export function duration(ms: number) {
  const total = Math.max(0, Math.round(ms / 1000));
  const unit = (value: number, name: "minute" | "second") =>
    new Intl.NumberFormat(i18n.language, { style: "unit", unit: name, unitDisplay: "short" }).format(value);
  return total < 60 ? unit(total, "second") : `${unit(Math.floor(total / 60), "minute")} ${unit(total % 60, "second")}`;
}

/**
 * What one Agent did in this conversation, derived from data the page already
 * has: the running turn right now and its last five results.
 */
export function MemberActivity({ turns, items, files, onJump }: {
  /** This Agent's turns in the conversation, oldest first. */
  turns: ActivityTurn[];
  items: ActivityItems;
  /** Files delivered on each turn's reply, by turn id. */
  files?: ReadonlyMap<string, number> | undefined;
  onJump: (turnId: string) => void;
}) {
  const current = turns.find((turn) => running(turn.state));
  const history = useMemo(
    () => turns.filter((turn) => !running(turn.state)).slice(-5).reverse(),
    [turns],
  );
  const [now, setNow] = useState(() => Date.now());
  // Only a local clock for the elapsed time; no extra requests.
  useEffect(() => {
    if (!current) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [current]);
  const latest = current ? [...(items.get(current.id)?.values() ?? [])].at(-1) : undefined;
  const pending = current ? Object.values(current.decisions).find((decision) => decision.decision === null && !decision.sent) : undefined;
  return (
    <section data-member-activity aria-label={t("memberActivity")} className="mt-4 rounded-xl border p-4 text-sm">
      <h4 className="mb-3 font-medium">{t("memberActivity")}</h4>
      {current && (
        <div data-activity-current className="mb-4 rounded-lg bg-primary/10 p-3">
          <p className="font-medium text-primary">{t("activityNow")}</p>
          <p className="mt-1 break-words">
            {pending
              ? `${t("activityWaitingApproval")}${pending.tool ?? ""}`
              : latest
                ? `${latest.name ?? ""}${latest.state ? ` · ${latest.state}` : ""}`
                : t("activityThinking")}
          </p>
          {current.created_at && (
            <p className="mt-1 text-xs text-muted-foreground">
              {t("activityElapsed")} {duration(now - current.created_at)}
            </p>
          )}
        </div>
      )}
      {history.length ? (
        <>
          <p className="mb-2 text-xs text-muted-foreground">{t("activityHistory")}</p>
          <ul className="space-y-1">
            {history.map((turn) => {
              const calls = items.get(turn.id)?.size ?? 0;
              const produced = files?.get(turn.id) ?? 0;
              return (
                <li key={turn.id}>
                  <button
                    type="button"
                    data-activity-turn={turn.id}
                    className="flex min-h-11 w-full flex-col items-start gap-0.5 rounded-lg px-2 py-2 text-left hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
                    title={t("activityJump")}
                    onClick={() => onJump(turn.id)}
                  >
                    <span className="flex w-full flex-wrap items-center gap-x-2">
                      <span className={turn.state === "completed" ? "" : "text-destructive"}>{t(turn.state)}</span>
                      {turn.created_at && (
                        <span className="text-xs text-muted-foreground">
                          {new Date(turn.created_at).toLocaleString(i18n.language, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}
                        </span>
                      )}
                      {turn.created_at && turn.updated_at && (
                        <span className="text-xs text-muted-foreground">{duration(turn.updated_at - turn.created_at)}</span>
                      )}
                      <span className="ml-auto text-xs text-muted-foreground">
                        {calls} {t("activityCalls")}{produced ? ` · ${produced} ${t("activityFiles")}` : ""}
                      </span>
                    </span>
                    {turn.state !== "completed" && turn.error && (
                      <span className="line-clamp-2 break-words text-xs text-muted-foreground">{turn.error}</span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </>
      ) : (
        !current && <p role="status" className="text-xs text-muted-foreground">{t("activityEmpty")}</p>
      )}
    </section>
  );
}
