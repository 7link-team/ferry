import { useState, type CSSProperties } from "react";
import { Dialog } from "radix-ui";
import {
  Bot,
  Check,
  ImagePlus,
  Palette,
  Paperclip,
  Send,
  SunMoon,
  UserRound,
  X,
} from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { conversationText as t } from "./resource-config";
import type { Attachment } from "./AttachmentView";
import everydayWallpaper from "./wallpapers/everyday.svg";
import cosmosWallpaper from "./wallpapers/cosmos.svg";
import botanicalWallpaper from "./wallpapers/botanical.svg";

export interface ChatAppearance {
  pattern: string;
  color: string;
  image: string | null;
}
export const defaultAppearance: ChatAppearance = {
  pattern: "plain",
  color: "",
  image: null,
};
export function conversationBackground(
  value: ChatAppearance | undefined,
  endpoint: string,
): CSSProperties {
  const v = value ?? defaultAppearance;
  const patterns: Record<string, string> = {
    // Retain saved pattern keys; only their visual assets and labels change.
    dots: `url("${everydayWallpaper}")`,
    grid: `url("${cosmosWallpaper}")`,
    diagonal: `url("${botanicalWallpaper}")`,
  };
  return {
    backgroundColor: /^#[\da-f]{6}$/i.test(v.color)
      ? v.color
      : "var(--background)",
    backgroundImage:
      v.image && /^\d+$/.test(v.image)
        ? `linear-gradient(color-mix(in oklab, var(--background) 25%, transparent), color-mix(in oklab, var(--background) 25%, transparent)), url("/api${endpoint}/files/${v.image}")`
        : (patterns[v.pattern] ?? "none"),
    backgroundSize: v.image ? "cover" : "400px 440px",
    backgroundPosition: "center",
  };
}

const colors = [
  "#e7ded3",
  "#d8e9df",
  "#dde5ee",
  "#e5dff0",
  "#f0dfe3",
  "#f1e7ce",
  "#d0d5d8",
  "#202c33",
];
export function ConversationAppearance({
  endpoint,
  value,
  onSaved,
  canUpload,
}: {
  endpoint: string;
  value: ChatAppearance | undefined;
  onSaved: (v: ChatAppearance) => void;
  canUpload: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState(value ?? defaultAppearance);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function upload(file: File | undefined) {
    if (!file) return;
    if (
      file.size > 5 * 1024 * 1024 ||
      !["image/png", "image/jpeg", "image/webp", "image/avif"].includes(
        file.type,
      )
    ) {
      setError(t("backgroundImageLimit"));
      return;
    }
    setBusy(true);
    setError("");
    try {
      const body = new FormData();
      body.append("file", file);
      const uploaded = await api<Attachment>(endpoint + "/files", {
        method: "POST",
        body,
      });
      setDraft((old) => ({ ...old, image: uploaded.file_id! }));
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  async function save() {
    setBusy(true);
    setError("");
    try {
      const saved = await api<ChatAppearance>(endpoint + "/appearance", {
        method: "PUT",
        body: JSON.stringify(draft),
      });
      onSaved(saved);
      setOpen(false);
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog.Root
      open={open}
      onOpenChange={(next) => {
        if (busy) return;
        setOpen(next);
        if (next) {
          setDraft(value ?? defaultAppearance);
          setError("");
        }
      }}
    >
      <Dialog.Trigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("chatAppearance")}
          title={t("chatAppearance")}
        >
          <Palette />
        </Button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/50" />
        <Dialog.Content
          data-wallpaper-dialog
          aria-describedby="chat-background-scope"
          className="fixed left-1/2 top-1/2 z-50 flex max-h-[90dvh] w-[min(54rem,calc(100vw-1.5rem))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-2xl border bg-background shadow-xl"
        >
          <div className="flex shrink-0 items-center justify-between gap-3 border-b px-5 py-3">
            <div className="min-w-0">
              <Dialog.Title className="font-semibold">
                {t("chatAppearance")}
              </Dialog.Title>
              <Dialog.Description
                id="chat-background-scope"
                className="mt-1 text-xs leading-relaxed text-muted-foreground"
              >
                {t("backgroundScope")}
              </Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <Button
                disabled={busy}
                variant="ghost"
                size="icon"
                className="size-11 shrink-0"
                aria-label={t("close")}
              >
                <X />
              </Button>
            </Dialog.Close>
          </div>
          <div className="grid min-h-0 overflow-y-auto md:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)]">
            <div className="order-2 min-w-0 space-y-6 p-4 md:order-1 md:p-5">
              <fieldset disabled={busy} className="min-w-0 space-y-3">
                <legend className="mb-3 text-sm font-medium">
                  {t("backgroundPattern")}
                </legend>
                <div className="grid grid-cols-2 gap-2">
                  {["plain", "dots", "grid", "diagonal"].map((pattern) => {
                    const selected = draft.pattern === pattern && !draft.image;
                    return (
                      <button
                        key={pattern}
                        type="button"
                        aria-pressed={selected}
                        onClick={() =>
                          setDraft({ ...draft, pattern, image: null })
                        }
                        className={`relative min-h-24 overflow-hidden rounded-xl border text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 ${selected ? "border-primary ring-2 ring-primary/30" : "border-border hover:border-foreground/40"}`}
                      >
                        <span
                          aria-hidden
                          className="block h-16"
                          style={{
                            ...conversationBackground(
                              { pattern, color: draft.color, image: null },
                              endpoint,
                            ),
                            backgroundSize: "200px 220px",
                          }}
                        />
                        <span className="block bg-card px-3 py-2 text-card-foreground">
                          {t(`background_${pattern}`)}
                        </span>
                        {selected && (
                          <span className="absolute right-2 top-2 flex size-5 items-center justify-center rounded-full bg-primary text-primary-foreground">
                            <Check className="size-3" />
                          </span>
                        )}
                      </button>
                    );
                  })}
                </div>
              </fieldset>
              <fieldset disabled={busy} className="min-w-0 space-y-3">
                <legend className="mb-3 text-sm font-medium">
                  {t("backgroundColor")}
                </legend>
                <div
                  className="flex flex-wrap gap-2"
                  role="group"
                  aria-label={t("backgroundColor")}
                >
                  <button
                    type="button"
                    aria-label={t("followTheme")}
                    title={t("followTheme")}
                    aria-pressed={!draft.color}
                    onClick={() => setDraft({ ...draft, color: "" })}
                    className={`flex size-11 items-center justify-center rounded-full border bg-background outline-none focus-visible:ring-2 focus-visible:ring-ring ${!draft.color ? "ring-2 ring-primary ring-offset-2 ring-offset-background" : "border-border"}`}
                  >
                    <SunMoon className="size-4" />
                  </button>
                  {colors.map((color) => (
                    <button
                      key={color}
                      type="button"
                      aria-label={`${t("backgroundColor")} ${color}`}
                      title={color}
                      aria-pressed={draft.color.toLowerCase() === color}
                      onClick={() => setDraft({ ...draft, color })}
                      style={{ backgroundColor: color }}
                      className={`flex size-11 items-center justify-center rounded-full border border-black/10 outline-none focus-visible:ring-2 focus-visible:ring-ring ${draft.color.toLowerCase() === color ? "ring-2 ring-primary ring-offset-2 ring-offset-background" : "hover:ring-2 hover:ring-ring/30"}`}
                    >
                      {draft.color.toLowerCase() === color && (
                        <span className="flex size-5 items-center justify-center rounded-full bg-background text-foreground shadow-sm">
                          <Check className="size-3" />
                        </span>
                      )}
                    </button>
                  ))}
                </div>
                <div className="flex items-center gap-3">
                  <label
                    htmlFor="chat-background-color"
                    className="min-w-0 flex-1 text-xs text-muted-foreground"
                  >
                    {t("backgroundCustomColor")}
                  </label>
                  <Input
                    id="chat-background-color"
                    type="color"
                    value={draft.color || colors[0]}
                    className="h-11 w-14 shrink-0 cursor-pointer p-1"
                    onChange={(e) =>
                      setDraft({ ...draft, color: e.target.value })
                    }
                  />
                </div>
              </fieldset>
              <div className="space-y-3 border-t pt-4">
                {canUpload && (
                  <label
                    className={`flex min-h-11 cursor-pointer items-center justify-center gap-2 rounded-lg border bg-background px-3 py-2 text-sm font-medium focus-within:ring-2 focus-within:ring-ring ${busy ? "pointer-events-none opacity-50" : "hover:bg-accent"}`}
                  >
                    <ImagePlus className="size-4 shrink-0" />
                    {t("backgroundUpload")}
                    <input
                      aria-label={t("backgroundUpload")}
                      className="sr-only"
                      type="file"
                      accept="image/png,image/jpeg,image/webp,image/avif"
                      disabled={busy}
                      onChange={(e) => {
                        const file = e.target.files?.[0];
                        e.target.value = "";
                        void upload(file);
                      }}
                    />
                  </label>
                )}
                {draft.image && (
                  <div className="flex items-center gap-3 rounded-lg border p-2">
                    <div
                      className="size-12 shrink-0 rounded-md border"
                      style={conversationBackground(draft, endpoint)}
                    />
                    <span className="min-w-0 flex-1 text-sm">
                      {t("backgroundImageSelected")}
                    </span>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="size-11 shrink-0"
                      disabled={busy}
                      aria-label={t("backgroundRemoveImage")}
                      onClick={() => setDraft({ ...draft, image: null })}
                    >
                      <X className="size-4" />
                    </Button>
                  </div>
                )}
              </div>
            </div>
            <div className="order-1 min-w-0 border-b bg-muted/30 p-3 md:order-2 md:border-b-0 md:border-l md:p-5">
              <div className="mb-2 flex items-center gap-2 text-xs font-medium text-muted-foreground">
                <span className="size-1.5 rounded-full bg-primary" />
                {t("backgroundLivePreview")}
              </div>
              <div
                data-wallpaper-preview
                className="flex min-h-64 flex-col overflow-hidden rounded-xl border shadow-sm md:sticky md:top-5 md:min-h-[30rem]"
                aria-label={t("preview")}
                style={conversationBackground(draft, endpoint)}
              >
                <div className="flex items-center gap-2 border-b bg-background px-3 py-2.5 text-sm">
                  <span className="flex size-7 items-center justify-center rounded-full bg-muted">
                    <Bot className="size-4" />
                  </span>
                  <span className="font-medium">{t("preview")}</span>
                </div>
                <div className="flex-1 space-y-4 px-3 py-5 md:py-8">
                  <div className="flex items-start gap-2">
                    <UserRound className="mt-2 size-4 shrink-0 text-muted-foreground" />
                    <div className="rounded-xl rounded-tl-sm border border-primary/20 bg-muted px-3 py-2.5 text-sm leading-relaxed text-foreground shadow-xs">
                      {t("backgroundPreviewUser")}
                    </div>
                  </div>
                  <div className="flex items-start gap-2">
                    <Bot className="mt-2 size-4 shrink-0 text-primary" />
                    <div className="rounded-xl rounded-tl-sm border bg-card px-3 py-2.5 text-sm leading-relaxed text-card-foreground shadow-xs">
                      {t("backgroundPreviewAgent")}
                    </div>
                  </div>
                </div>
                <div className="px-3 pb-3 pt-6">
                  <div className="flex items-center gap-2 rounded-xl border bg-background px-3 py-3 text-muted-foreground shadow-sm">
                    <Paperclip className="size-4 shrink-0" />
                    <span className="min-w-0 flex-1 truncate text-xs">
                      {t("messagePlaceholder")}
                    </span>
                    <span className="flex size-7 items-center justify-center rounded-full bg-primary text-primary-foreground">
                      <Send className="size-3" />
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </div>
          <div className="shrink-0 space-y-3 border-t bg-background p-4 pb-[max(1rem,env(safe-area-inset-bottom))]">
            {error && (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            )}
            <div className="flex gap-3">
              <Button
                variant="outline"
                disabled={busy}
                className="min-h-11 flex-1"
                onClick={() => {
                  setDraft(defaultAppearance);
                  setError("");
                }}
              >
                {t("resetBackground")}
              </Button>
              <Button
                disabled={busy}
                className="min-h-11 flex-1"
                onClick={() => void save()}
              >
                {t("saveBackground")}
              </Button>
            </div>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
