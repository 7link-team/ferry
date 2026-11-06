import { useEffect, useId, useRef, useState } from "react";
import { commonText } from "../shared/i18n/i18n";
import { Button } from "../shared/ui/button";
import { Input } from "../shared/ui/input";

export type LoginChallengeData = {
  required: boolean; kind?: "image" | "math" | "select" | "slider" | "turnstile";
  id?: string; image?: string; choices?: string[]; site_key?: string;
  slider_path?: { start_y: number; slope: number };
};
export type LoginProof = { login_challenge_id?: string; login_challenge_answer?: string; turnstile_token?: string };
type Turnstile = {
  render(element: HTMLElement, options: Record<string, unknown>): string;
  remove(id: string): void;
};
function turnstile() { return (window as unknown as {turnstile?: Turnstile}).turnstile; }
const text = (key: string) => commonText(`resource-domain:loginGuard.${key}`);

export function LoginChallenge({ challenge, onProof, onRefresh, onVerify, busy }: {
  challenge: LoginChallengeData; onProof: (proof: LoginProof) => void; onRefresh: () => void; onVerify?: (proof: LoginProof) => void; busy: boolean;
}) {
  const container = useRef<HTMLDivElement>(null);
  const [answer, setAnswer] = useState("");
  const [dragging, setDragging] = useState(false);
  const sliderHelpId = useId();
  const [unavailable, setUnavailable] = useState(false);
  useEffect(() => { setAnswer(""); setDragging(false); setUnavailable(false); onProof({}); }, [challenge, onProof]);
  useEffect(() => {
    if (challenge.kind !== "turnstile") return;
    let live = true, widget: string | undefined;
    const render = () => {
      const engine = turnstile();
      if (!live || !engine || !container.current) return;
      try { widget = engine.render(container.current, {
        sitekey: challenge.site_key, action: "login", size: "flexible",
        callback: (token: string) => onProof({ turnstile_token: token }),
        "expired-callback": () => onProof({}),
        "error-callback": () => { onProof({}); setUnavailable(true); },
      }); } catch { setUnavailable(true); }
    };
    let script = document.getElementById("login-turnstile-script") as HTMLScriptElement | null;
    const failed = () => { if (live) {setUnavailable(true);script?.remove();} };
    if (turnstile()) render();
    else {
      if (!script) {
        script = document.createElement("script"); script.id = "login-turnstile-script";
        script.src = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
        script.async = true; document.head.appendChild(script);
      }
      script.addEventListener("load", render); script.addEventListener("error", failed);
    }
    return () => {
      live = false; script?.removeEventListener("load", render); script?.removeEventListener("error", failed);
      if (widget) turnstile()?.remove(widget);
    };
  }, [challenge, onProof]);
  const update = (value: string) => {
    setAnswer(value); onProof(challenge.kind === "select" && value.length !== 3 ? {} : { login_challenge_id: challenge.id??"", login_challenge_answer: value });
  };
  const release=()=>{setDragging(false);if(!busy&&answer)onVerify?.({login_challenge_id:challenge.id??"",login_challenge_answer:answer});};
  return <div className="space-y-3 rounded-md border p-3" aria-label={text("title")}>
    <p className="text-sm text-muted-foreground">{text(challenge.kind ?? "image")}</p>
    {challenge.kind === "turnstile" ? <div ref={container} className="min-h-16 min-w-0" /> : <>
      <div className="relative mx-auto w-full max-w-[280px]">
        <img src={challenge.image} alt={text(challenge.kind ?? "image")} width={280} height={80} className="h-auto w-full rounded border" />
        {challenge.kind === "slider" && <span aria-hidden className="pointer-events-none absolute h-[30%] w-[8.57%] rounded-sm border-2 border-primary bg-primary/30" style={{left: `${Number(answer || 0) / 280 * 100}%`, top: `${((challenge.slider_path?.start_y ?? 28) + Number(answer || 0) * (challenge.slider_path?.slope ?? 0)) / 80 * 100}%`}} />}
      </div>
      {challenge.kind === "select" ? <div className="space-y-2">
        <div className="grid grid-cols-4 gap-2">{challenge.choices?.map(choice => <Button key={choice} type="button" aria-pressed={answer.includes(choice)} variant={answer.includes(choice) ? "secondary" : "outline"} disabled={busy || answer.includes(choice) || answer.length === 3} className="h-11 min-w-0 text-base" onClick={() => update(answer + choice)}>{choice}</Button>)}</div>
        <div className="flex items-center justify-between gap-2 text-sm"><span>{text("selectionOrder")} ({answer.length}/3)</span><Button type="button" variant="ghost" disabled={busy || !answer} onClick={() => update("")}>{text("clear")}</Button></div>
        <output aria-live="polite" aria-label={text("selectionOrder")} className="grid grid-cols-3 gap-2">{[0, 1, 2].map(index => <span key={index} className="flex h-11 items-center justify-center gap-2 rounded-md border bg-muted/30"><span className="text-xs text-muted-foreground">{index + 1}.</span><span className="text-base font-medium">{answer[index] || text("unselected")}</span></span>)}</output>
        <p className="text-xs text-muted-foreground">{text("submitHint")}</p>
      </div> : challenge.kind === "slider" ? <div className="space-y-2">
        <label className="block text-sm">
          <span className="sr-only">{text("slider")}</span><input type="range" min={0} max={256} step={1} value={answer || 0} disabled={busy} aria-describedby={sliderHelpId} className="mx-auto block h-11 w-full max-w-[280px] cursor-ew-resize appearance-none rounded-md border bg-muted accent-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 [&::-webkit-slider-thumb]:h-11 [&::-webkit-slider-thumb]:w-11 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-md [&::-webkit-slider-thumb]:border [&::-webkit-slider-thumb]:border-primary [&::-webkit-slider-thumb]:bg-primary [&::-moz-range-thumb]:h-10 [&::-moz-range-thumb]:w-10 [&::-moz-range-thumb]:rounded-md [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-primary" onPointerDown={() => setDragging(true)} onPointerUp={release} onKeyUp={event => { if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(event.key)) release(); }} onPointerCancel={() => setDragging(false)} onBlur={() => setDragging(false)} onChange={event => update(event.target.value)} />
        </label>
        <p id={sliderHelpId} role="status" className="text-sm text-muted-foreground">{text(dragging ? "sliderDragging" : answer ? "sliderReady" : "sliderIdle")}</p>
      </div> : <Input aria-label={text("answer")} value={answer} maxLength={32} disabled={busy} autoComplete="off" inputMode={challenge.kind === "math" ? "numeric" : "text"} placeholder={text("answer")} onChange={event => update(event.target.value)} />}
    </>}
    {unavailable && <p role="alert" className="text-sm text-destructive">{text("unavailable")}</p>}
    <Button type="button" variant="ghost" disabled={busy} onClick={onRefresh}>{text("refresh")}</Button>
  </div>;
}
