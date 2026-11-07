import { Columns2, IndentIncrease, Maximize2, Minimize2, Upload } from "lucide-react";
import { Dialog } from "radix-ui";
import { useEffect, useRef, useState } from "react";
import { CodeEditor } from "../../shared/editor/CodeEditor";
import { executionText, useCommonTranslation } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { formatActionYaml } from "./actions-workflow";

const prettyIfValid = (source: string) => { try { return formatActionYaml(source); } catch { return source; } };

export function YamlEditor({ value, originalValue, readOnly, fillHeight, onChange, label = "GitHub Actions YAML", path = "file:///pipeline.yaml" }: {
  value: string;
  originalValue?: string;
  readOnly: boolean;
  fillHeight: boolean;
  onChange: (value: string) => void;
  label?: string;
  path?: string;
}) {
  useCommonTranslation();
  const fileInput = useRef<HTMLInputElement>(null);
  const initial = useRef(originalValue ?? value);
  const emitted = useRef(value);
  const [displayValue, setDisplayValue] = useState(() => prettyIfValid(value));
  const [formatError, setFormatError] = useState("");
  useEffect(() => {
    // Reformat external loads/form changes, never each keystroke from Monaco.
    if (value !== emitted.current) { emitted.current = value; setDisplayValue(prettyIfValid(value)); setFormatError(""); }
  }, [value]);
  const update = (next: string) => { emitted.current = next; setDisplayValue(next); setFormatError(""); onChange(next); };
  const [expanded, setExpanded] = useState(false);
  const [compare, setCompare] = useState(false);
  const ownsFullscreen = useRef(false);
  const close = () => {
    setExpanded(false);
    if (ownsFullscreen.current && document.fullscreenElement === document.documentElement) void document.exitFullscreen().catch(() => {});
    ownsFullscreen.current = false;
  };
  useEffect(() => {
    const changed = () => { if (ownsFullscreen.current && !document.fullscreenElement) { ownsFullscreen.current = false; setExpanded(false); } };
    document.addEventListener("fullscreenchange", changed);
    return () => { document.removeEventListener("fullscreenchange", changed); if (ownsFullscreen.current && document.fullscreenElement === document.documentElement) void document.exitFullscreen().catch(() => {}); };
  }, []);
  const toolbar = <div className="flex flex-wrap items-center justify-between gap-2">
    <span className="text-sm font-medium">{label}</span><div className="flex flex-wrap items-center gap-2">
      {!readOnly && <Button type="button" variant="outline" size="sm" onClick={() => fileInput.current?.click()}><Upload />{executionText("editor.importYaml")}</Button>}
      {!readOnly && <Button type="button" variant="outline" size="sm" onClick={() => {
        try { update(formatActionYaml(displayValue)); } catch (cause) { setFormatError(cause instanceof Error ? cause.message : String(cause)); }
      }}><IndentIncrease />{executionText("editor.formatYaml")}</Button>}
      <Button type="button" variant={compare ? "secondary" : "outline"} size="sm" aria-pressed={compare} onClick={() => setCompare(value => !value)}><Columns2 />{executionText("editor.compare")}</Button>
      <Button type="button" variant="outline" size="sm" onClick={() => {
        if (expanded) { close(); return; }
        setExpanded(true);
        if (!document.fullscreenElement && document.documentElement.requestFullscreen) {
          ownsFullscreen.current = true;
          void document.documentElement.requestFullscreen().catch(() => { ownsFullscreen.current = false; });
        }
      }}>{expanded ? <Minimize2 /> : <Maximize2 />}{executionText(expanded ? "editor.exitFullscreen" : "editor.fullscreen")}</Button>
    </div>
  </div>;
  const editor = <div className="min-h-0 flex-1 overflow-hidden rounded-md border border-input">
    {compare && <div className="grid grid-cols-2 border-b bg-muted/40 text-center text-xs"><span className="p-2">{executionText("editor.original")}</span><span className="p-2">{executionText("editor.modified")}</span></div>}
    <div className={compare ? "h-[calc(100%-2rem)]" : "h-full"}><CodeEditor path={path} language="yaml" label={label} value={displayValue} onChange={update} readOnly={readOnly} {...(compare ? { originalValue: originalValue ?? initial.current } : {})} /></div>
  </div>;
  return <div className={fillHeight ? "flex min-h-0 flex-1 flex-col gap-2" : "flex h-[520px] min-h-[360px] flex-col gap-2"}>
    <input ref={fileInput} className="hidden" type="file" accept=".yml,.yaml,text/yaml" aria-label={executionText("editor.selectYamlFile")} onChange={async event => { const file = event.target.files?.[0]; if (file) onChange(await file.text()); event.target.value = ""; }} />
    {!expanded && <>{toolbar}{formatError && <p role="alert" className="text-sm text-destructive">{formatError}</p>}{editor}</>}
    <Dialog.Root open={expanded} onOpenChange={open => { if (!open) close(); }}><Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-[100] bg-background" />
      <Dialog.Content aria-describedby={undefined} className="fixed inset-0 z-[101] flex h-dvh w-screen flex-col gap-3 overflow-hidden bg-background p-3 sm:p-4">
        <Dialog.Title className="sr-only">{executionText("editor.fullscreen")}</Dialog.Title>
        {toolbar}{formatError && <p role="alert" className="text-sm text-destructive">{formatError}</p>}{editor}
      </Dialog.Content>
    </Dialog.Portal></Dialog.Root>
  </div>;
}
