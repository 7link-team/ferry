import { useEffect, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { base } from "../../shared/api/resource";
import { Button } from "../../shared/ui/button";
import { InlineAlert } from "../../shared/ui/inline-alert";
import { conversationText as t } from "./resource-config";

/** Room-wide controls any participant may use: Agent relay and adding their own Agent. */
export function RoomControls({ workspace, endpoint, relayEnabled, canRun, onChanged }: {
  workspace: string; endpoint: string; relayEnabled: boolean; canRun: boolean;
  onChanged: () => Promise<void>;
}) {
  const [options, setOptions] = useState<{ id: string; name: string }[]>([]);
  const [choice, setChoice] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!canRun) return;
    let live = true;
    void api<{ items: { id: string; name: string; state: string; visibility: string }[] }>(`${base(workspace, "ai-agents")}?page_size=100`)
      .then(result => { if (live) setOptions(result.items.filter(a => a.state === "enabled" && a.visibility === "workspace")); })
      .catch(() => { if (live) setOptions([]); });
    return () => { live = false; };
  }, [workspace, canRun]);
  if (!canRun) return null;
  async function act(action: () => Promise<unknown>) {
    setBusy(true); setError("");
    try { await action(); await onChanged(); } catch (cause) { setError(errorText(cause)); }
    finally { setBusy(false); }
  }
  return <section data-room-controls className="mt-5 space-y-3 border-t pt-4">
    <label className="flex min-h-11 items-center gap-3 text-sm">
      <input type="checkbox" className="size-5" checked={relayEnabled} disabled={busy}
        onChange={event => void act(() => api(`${endpoint}/relay`, { method: "PATCH", body: JSON.stringify({ enabled: event.target.checked }) }))} />
      <span>{t("relayEnabled")}</span>
    </label>
    {!!options.length && <div className="flex flex-wrap gap-2">
      <select aria-label={t("addMyAgent")} className="min-h-11 min-w-0 flex-1 basis-40 rounded-md border bg-background px-2 text-base sm:text-sm" value={choice} disabled={busy} onChange={event => setChoice(event.target.value)}>
        <option value="">{t("addMyAgent")}</option>
        {options.map(option => <option key={option.id} value={option.id}>{option.name}</option>)}
      </select>
      <Button variant="outline" className="min-h-11" disabled={busy || !choice}
        onClick={() => void act(async () => { await api(`${endpoint}/agents`, { method: "POST", body: JSON.stringify({ agent_id: choice }) }); setChoice(""); })}>
        {t("addMyAgent")}
      </Button>
    </div>}
    {error && <InlineAlert>{error}</InlineAlert>}
  </section>;
}

/** Shown while Agent relays wait for a member (hop limit reached or relay off). */
export function HeldRelays({ endpoint, onChanged }: { endpoint: string; onChanged: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  return <div role="status" data-held-relays className="mx-3 mb-2 flex flex-wrap items-center gap-2 rounded-xl border border-primary/40 bg-primary/5 px-3 py-2 text-sm sm:mx-6">
    <span className="min-w-0 flex-1 basis-48">{t("relayHeld")}</span>
    <Button size="sm" className="min-h-11" disabled={busy} onClick={() => {
      setBusy(true); setError("");
      void api(`${endpoint}/relay/continue`, { method: "POST" })
        .then(onChanged)
        .catch(cause => setError(errorText(cause)))
        .finally(() => setBusy(false));
    }}>{t("relayContinue")}</Button>
    {error && <InlineAlert className="w-full">{error}</InlineAlert>}
  </div>;
}
