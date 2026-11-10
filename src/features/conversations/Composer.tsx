import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import {
  AtSign,
  ChevronDown,
  Grip,
  Mic,
  Paperclip,
  Reply,
  Square,
  X,
} from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { AutoForm } from "../../shared/forms/auto-form";
import { Button } from "../../shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from "../../shared/ui/dropdown-menu";
import { conversationText as t } from "./resource-config";
import type { Attachment } from "./AttachmentView";
import {
  mentionAt,
  mentionRanges,
  selectMentionDeletion,
  MentionSuggestions,
  type MentionToken,
  type MentionOption,
} from "./MentionSuggestions";
export interface ComposerDraft {
  prompt: string;
  attachments: Attachment[];
  targets?: string[];
  mentions?: string[];
  reply_to?: string;
}
function saved(key: string, fallback: string) {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}
function remember(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* Optional local preference. */
  }
}
export function Composer({
  endpoint,
  draft,
  onChange,
  onSend,
  disabled,
  busy,
  current,
  onStop,
  group,
  agents,
  people = [],
  reply,
  onCancelReply,
}: {
  reply?: { name: string; text: string } | undefined;
  onCancelReply?: () => void;
  endpoint: string;
  draft: ComposerDraft;
  onChange: (value: ComposerDraft) => void;
  onSend: () => Promise<void>;
  disabled: boolean;
  busy: boolean;
  current?: { state: string } | undefined;
  onStop: () => void;
  group: boolean;
  agents: { id: string; name: string; state: string }[];
  people?: { id: string; name: string }[];
}) {
  const [error, setError] = useState("");
  const [uploading, setUploading] = useState(false);
  const [mode, setMode] = useState(() => saved("ui.chat.sendKey", "enter"));
  const [height, setHeight] = useState(() =>
    Math.min(
      360,
      Math.max(80, Number(saved("ui.chat.composerHeight", "112")) || 112),
    ),
  );
  const [dragging, setDragging] = useState(false);
  const [mention, setMention] = useState<MentionToken | null>(null);
  const [activeMention, setActiveMention] = useState(0);
  const dismissedMention = useRef("");
  const removedMentions = useRef(new Map<string, MentionOption>());
  const [recording, setRecording] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const textarea = useRef<HTMLTextAreaElement>(null);
  const pendingSelection = useRef<number | null>(null);
  useLayoutEffect(() => {
    if (pendingSelection.current === null || !textarea.current) return;
    const caret = pendingSelection.current;
    pendingSelection.current = null;
    textarea.current.focus();
    textarea.current.setSelectionRange(caret, caret);
  });
  const recorder = useRef<MediaRecorder | null>(null);
  const disposed = useRef(false);
  const recordingTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const resizing = useRef<{ y: number; height: number } | null>(null);
  const latest = useRef(draft);
  latest.current = draft;
  useEffect(() => {
    removedMentions.current.clear();
  }, [endpoint]);
  useEffect(() => {
    disposed.current = false;
    return () => {
      disposed.current = true;
      clearTimeout(recordingTimer.current);
      const r = recorder.current;
      if (r?.state === "recording") r.stop();
      r?.stream.getTracks().forEach((track) => track.stop());
    };
  }, []);
  const locked = disabled || busy || uploading || (!group && !!current);
  async function attach(files: File[]) {
    if (!files.length || locked || recording) return;
    setError("");
    setUploading(true);
    const uploaded: Attachment[] = [];
    try {
      if (
        files.length + latest.current.attachments.length > 10 ||
        files.some((f) => f.size > 32 * 1024 * 1024)
      )
        throw Error(t("attachmentLimit"));
      for (const file of files) {
        const body = new FormData();
        body.append("file", file);
        const a = await api<Attachment>(endpoint + "/files", {
          method: "POST",
          body,
        });
        uploaded.push(a);
      }
    } catch (cause) {
      if (!disposed.current) setError(errorText(cause));
    } finally {
      if (!disposed.current) {
        if (uploaded.length)
          onChange({
            ...latest.current,
            attachments: [...latest.current.attachments, ...uploaded],
          });
        setUploading(false);
      }
    }
  }
  async function remove(index: number) {
    const a = draft.attachments[index];
    if (!a) return;
    setError("");
    try {
      if (a.file_id)
        await api(endpoint + "/files/" + encodeURIComponent(a.file_id), {
          method: "DELETE",
        });
      onChange({
        ...draft,
        attachments: draft.attachments.filter((_, i) => i !== index),
      });
    } catch (cause) {
      setError(errorText(cause));
    }
  }
  async function toggleRecording() {
    const current = recorder.current;
    if (current?.state === "recording") {
      current.stop();
      return;
    }
    if (
      !navigator.mediaDevices?.getUserMedia ||
      typeof MediaRecorder === "undefined"
    ) {
      setError(t("recordingUnsupported"));
      return;
    }
    let acquired: MediaStream | undefined;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      acquired = stream;
      if (disposed.current) {
        stream.getTracks().forEach((t) => t.stop());
        return;
      }
      const mime = ["audio/webm", "audio/ogg", "audio/mp4"].find((m) =>
        MediaRecorder.isTypeSupported(m),
      );
      const r = new MediaRecorder(
        stream,
        mime ? { mimeType: mime } : undefined,
      );
      recorder.current = r;
      const chunks: Blob[] = [];
      let size = 0,
        tooLarge = false,
        failed = false;
      r.ondataavailable = (e) => {
        size += e.data.size;
        if (size > 32 * 1024 * 1024) {
          tooLarge = true;
          if (r.state === "recording") r.stop();
        } else chunks.push(e.data);
      };
      r.onstop = () => {
        clearTimeout(recordingTimer.current);
        stream.getTracks().forEach((t) => t.stop());
        recorder.current = null;
        if (disposed.current) return;
        setRecording(false);
        if (failed) {
          setError(t("recordingFailed"));
          return;
        }
        if (tooLarge) {
          setError(t("attachmentLimit"));
          return;
        }
        const type = r.mimeType.split(";")[0] || "audio/webm",
          ext = type.includes("mp4")
            ? "m4a"
            : type.includes("ogg")
              ? "ogg"
              : "webm";
        const file = new File(chunks, `voice-${Date.now()}.${ext}`, { type });
        // Recording has stopped; upload through the same bounded file endpoint.
        const body = new FormData();
        body.append("file", file);
        setUploading(true);
        void api<Attachment>(endpoint + "/files", { method: "POST", body })
          .then((a) => {
            if (!disposed.current)
              onChange({
                ...latest.current,
                attachments: [...latest.current.attachments, a],
              });
          })
          .catch((cause) => {
            if (!disposed.current) setError(errorText(cause));
          })
          .finally(() => {
            if (!disposed.current) setUploading(false);
          });
      };
      r.onerror = () => {
        failed = true;
        setError(t("recordingFailed"));
        if (r.state !== "inactive") r.stop();
      };
      r.start(1000);
      setRecording(true);
      setError("");
      recordingTimer.current = setTimeout(
        () => {
          if (r.state === "recording") r.stop();
        },
        5 * 60 * 1000,
      );
    } catch {
      acquired?.getTracks().forEach((track) => track.stop());
      recorder.current = null;
      setRecording(false);
      setError(t("microphoneDenied"));
    }
  }
  const targets = draft.targets ?? [];
  const mentions = draft.mentions ?? [];
  const mentionQuery = mention?.query.toLocaleLowerCase() ?? "";
  const participants = [
    ...agents.map((a) => ({ ...a, human: false })),
    ...people.map((p) => ({ ...p, state: "active", human: true })),
  ];
  const mentionOptions = participants.filter((a) =>
    a.name.toLocaleLowerCase().includes(mentionQuery),
  );
  const selectedParticipants = participants.filter((a) =>
    (a.human ? mentions : targets).includes(a.id),
  );
  useEffect(() => {
    const input = textarea.current;
    if (!input || !group || locked || recording) return;
    const before = (event: InputEvent) => {
      if (
        !event.isComposing &&
        ["deleteContentBackward", "deleteContentForward"].includes(
          event.inputType,
        )
      )
        selectMentionDeletion(
          input,
          selectedParticipants,
          event.inputType === "deleteContentBackward",
        );
    };
    input.addEventListener("beforeinput", before);
    return () => input.removeEventListener("beforeinput", before);
  });
  function changePrompt(prompt: string, inputType?: string) {
    if (!group) {
      onChange({ ...draft, prompt });
      return;
    }
    const previous = mentionRanges(draft.prompt, selectedParticipants);
    const remaining = mentionRanges(prompt, selectedParticipants);
    const key = (a: MentionOption) => `${a.human}/${a.id}`;
    const removed = previous
      .filter((r) => !remaining.some((n) => key(n.option) === key(r.option)))
      .map((r) => r.option);
    for (const option of removed)
      removedMentions.current.set(key(option), option);
    const restored =
      inputType === "historyUndo" || inputType === "historyRedo"
        ? mentionRanges(
            prompt,
            participants.filter((a) => removedMentions.current.has(key(a))),
          )
            .map((r) => r.option)
            .filter((a) => a.state === "active")
        : [];
    onChange({
      ...draft,
      prompt,
      targets: [
        ...new Set([
          ...targets.filter(
            (id) => !removed.some((a) => !a.human && a.id === id),
          ),
          ...restored.filter((a) => !a.human).map((a) => a.id),
        ]),
      ],
      mentions: [
        ...new Set([
          ...mentions.filter(
            (id) => !removed.some((a) => a.human && a.id === id),
          ),
          ...restored.filter((a) => a.human).map((a) => a.id),
        ]),
      ],
    });
  }
  const effectiveMention =
    mentionOptions[activeMention]?.state === "active"
      ? activeMention
      : mentionOptions.findIndex((a) => a.state === "active");
  function toggleAgent(id: string) {
    removedMentions.current.delete(`false/${id}`);
    onChange({
      ...draft,
      targets: targets.includes(id)
        ? targets.filter((v) => v !== id)
        : [...targets, id],
    });
  }
  function updateMention(input: HTMLTextAreaElement, changed = false) {
    if (!group || locked || recording) return;
    const token = mentionAt(
      input.value,
      input.selectionStart,
      input.selectionEnd,
    );
    if (changed) dismissedMention.current = "";
    setMention(
      token && JSON.stringify(token) !== dismissedMention.current
        ? token
        : null,
    );
    if (changed || token?.query !== mention?.query) setActiveMention(0);
  }
  function dismissMention() {
    dismissedMention.current = JSON.stringify(mention);
    setMention(null);
  }
  function pickAgent(a: MentionOption) {
    if (!mention || a.state !== "active") return;
    const after = draft.prompt.slice(mention.end).replace(/^[^\s@]*/u, "");
    const prefix = draft.prompt.slice(0, mention.start) + `@${a.name}`;
    pendingSelection.current = prefix.length + 1;
    onChange({
      ...draft,
      prompt: prefix + (after.startsWith(" ") ? "" : " ") + after,
      ...(a.human
        ? { mentions: mentions.includes(a.id) ? mentions : [...mentions, a.id] }
        : { targets: targets.includes(a.id) ? targets : [...targets, a.id] }),
    });
    setMention(null);
  }
  function openMentions() {
    const input = textarea.current;
    if (!input) return;
    const start = input.selectionStart,
      end = input.selectionEnd;
    const existing = mentionAt(draft.prompt, start, end);
    dismissedMention.current = "";
    if (existing) {
      setMention(existing);
      input.focus();
      return;
    }
    const insert = `${start && !/\s$/.test(draft.prompt.slice(0, start)) ? " " : ""}@`;
    const prompt =
      draft.prompt.slice(0, start) + insert + draft.prompt.slice(end);
    pendingSelection.current = start + insert.length;
    onChange({
      ...draft,
      prompt,
    });
    setActiveMention(0);
    setMention(mentionAt(prompt, start + insert.length));
  }
  function keyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
    if (
      group &&
      !locked &&
      !recording &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.altKey &&
      (e.key === "Backspace" || e.key === "Delete")
    ) {
      selectMentionDeletion(
        e.currentTarget,
        selectedParticipants,
        e.key === "Backspace",
      );
      return;
    }
    if (e.key === "Escape") {
      if (mention) e.preventDefault();
      dismissMention();
      return;
    }
    if (
      mention &&
      !locked &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.altKey &&
      !e.shiftKey
    ) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const available = mentionOptions
          .map((a, i) => (a.state === "active" ? i : -1))
          .filter((i) => i >= 0);
        const index = available.indexOf(effectiveMention),
          delta = e.key === "ArrowDown" ? 1 : -1;
        setActiveMention(
          available[(index + delta + available.length) % available.length] ?? 0,
        );
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const option = mentionOptions[effectiveMention];
        if (option) pickAgent(option);
        return;
      }
    }
    if (
      e.key !== "Enter" ||
      e.nativeEvent.isComposing ||
      e.keyCode === 229 ||
      e.altKey
    )
      return;
    if (e.shiftKey) return;
    const modifier = e.ctrlKey || e.metaKey;
    const send = mode === "enter" ? !modifier : modifier;
    if (send) {
      e.preventDefault();
      if (
        !locked &&
        !recording &&
        (draft.prompt.trim() || draft.attachments.length)
      )
        void onSend();
    } else if (modifier) {
      e.preventDefault();
      const node = e.currentTarget;
      const start = node.selectionStart,
        end = node.selectionEnd;
      pendingSelection.current = start + 1;
      changePrompt(
        draft.prompt.slice(0, start) + "\n" + draft.prompt.slice(end),
      );
    }
  }
  function setComposerHeight(value: number) {
    const h = Math.round(
      Math.max(80, Math.min(value, Math.min(440, window.innerHeight * 0.45))),
    );
    setHeight(h);
    remember("ui.chat.composerHeight", String(h));
  }
  return (
    <AutoForm
      onSubmit={(e) => {
        e.preventDefault();
        if (!locked && !recording) void onSend();
      }}
      className="mx-auto w-full max-w-4xl space-y-2"
    >
      <div
        data-conversation-composer
        className={`relative isolate overflow-hidden rounded-xl border bg-background shadow-sm transition-colors focus-within:border-ring focus-within:ring-1 focus-within:ring-ring/20 ${dragging ? "border-primary bg-muted" : "border-input"}`}
        onDragOver={(e) => {
          if (!locked) {
            e.preventDefault();
            setDragging(true);
          }
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node))
            setDragging(false);
        }}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          void attach(Array.from(e.dataTransfer.files));
        }}
      >
        <div
          role="slider"
          tabIndex={0}
          aria-label={t("resizeComposer")}
          aria-orientation="vertical"
          aria-valuemin={80}
          aria-valuemax={Math.min(440, window.innerHeight * 0.45)}
          aria-valuenow={height}
          className="absolute right-1 top-1 z-10 flex size-7 touch-none cursor-ns-resize items-center justify-center rounded text-muted-foreground hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
          onPointerDown={(e) => {
            e.preventDefault();
            resizing.current = { y: e.clientY, height };
            e.currentTarget.setPointerCapture(e.pointerId);
          }}
          onPointerMove={(e) => {
            if (resizing.current)
              setComposerHeight(
                resizing.current.height + resizing.current.y - e.clientY,
              );
          }}
          onPointerUp={() => {
            resizing.current = null;
          }}
          onPointerCancel={() => {
            resizing.current = null;
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowUp" || e.key === "ArrowDown") {
              e.preventDefault();
              setComposerHeight(height + (e.key === "ArrowUp" ? 24 : -24));
            }
          }}
        >
          <Grip className="size-3.5" />
        </div>
        {reply && (
          <div data-conversation-reply className="m-2 mr-9 flex min-w-0 items-center gap-2 rounded-lg border-l-2 border-primary bg-muted/70 pl-3">
            <Reply aria-hidden="true" className="size-4 shrink-0 text-primary" />
            <div className="min-w-0 flex-1 py-2 text-sm">
              <p className="truncate font-medium text-primary">{t("replyingTo")} {reply.name}</p>
              <p className="line-clamp-2 whitespace-pre-wrap break-words text-muted-foreground">{reply.text}</p>
            </div>
            <Button type="button" size="icon" variant="ghost" className="size-11 shrink-0" aria-label={t("cancelReply")} onClick={onCancelReply}>
              <X className="size-4" />
            </Button>
          </div>
        )}
        <label htmlFor="conversation-message" className="sr-only">
          {t("prompt")}
        </label>
        <textarea
          ref={textarea}
          id="conversation-message"
          value={draft.prompt}
          disabled={locked || recording}
          style={{ height, maxHeight: "45dvh" }}
          placeholder={t(group ? "groupPlaceholder" : "messagePlaceholder")}
          onKeyDown={keyDown}
          role={group ? "combobox" : undefined}
          aria-haspopup={group ? "listbox" : undefined}
          aria-autocomplete={group ? "list" : undefined}
          aria-expanded={group ? !!mention : undefined}
          aria-controls={mention ? "conversation-mention-options" : undefined}
          aria-activedescendant={
            mention && mentionOptions[effectiveMention]
              ? `conversation-mention-${effectiveMention}`
              : undefined
          }
          onSelect={(e) => updateMention(e.currentTarget)}
          onBlur={dismissMention}
          onCompositionStart={() => setMention(null)}
          onCompositionEnd={(e) => updateMention(e.currentTarget, true)}
          onCut={(e) => {
            if (
              group &&
              !locked &&
              !recording &&
              e.currentTarget.selectionStart !== e.currentTarget.selectionEnd
            )
              selectMentionDeletion(
                e.currentTarget,
                selectedParticipants,
                true,
              );
          }}
          onChange={(e) => {
            changePrompt(
              e.target.value,
              (e.nativeEvent as InputEvent).inputType,
            );
            if (!(e.nativeEvent as InputEvent).isComposing)
              updateMention(e.target, true);
          }}
          onPaste={(e) => {
            const files = Array.from(e.clipboardData.files);
            if (files.length) {
              e.preventDefault();
              void attach(files);
            }
          }}
          className="block min-h-20 w-full resize-none border-0 bg-transparent px-3 py-3 pr-9 text-sm leading-6 shadow-none outline-none focus:ring-0 disabled:opacity-50"
        />
        {!!draft.attachments.length && (
          <div className="flex max-h-36 flex-wrap gap-2 overflow-auto px-3 pb-3">
            {draft.attachments.map((a, i) => (
              <div
                key={a.file_id ?? i}
                className="flex min-w-0 max-w-full items-center gap-1 rounded-md border bg-muted/40 py-1 pl-2 text-xs"
              >
                <Paperclip className="size-3 shrink-0" />
                <span className="max-w-48 truncate">{a.name}</span>
                <Button
                  size="icon"
                  type="button"
                  variant="ghost"
                  className="size-6"
                  disabled={locked || recording}
                  aria-label={`${t("remove")} ${a.name}`}
                  onClick={() => void remove(i)}
                >
                  <X className="size-3" />
                </Button>
              </div>
            ))}
          </div>
        )}
        {group && (
          <div className="flex flex-wrap gap-1 px-3 pb-2">
            {agents
              .filter((a) => targets.includes(a.id))
              .map((a) => (
                <Button
                  key={a.id}
                  type="button"
                  size="sm"
                  variant="secondary"
                  className="min-w-0 max-w-full"
                  disabled={locked}
                  onClick={() => toggleAgent(a.id)}
                >
                  <span className="truncate" title={a.name}>
                    @{a.name}
                  </span>
                  <X className="size-3 shrink-0" />
                </Button>
              ))}
            {people
              .filter((p) => mentions.includes(p.id))
              .map((p) => (
                <Button
                  key={p.id}
                  type="button"
                  size="sm"
                  variant="secondary"
                  className="min-w-0 max-w-full"
                  disabled={locked}
                  onClick={() => {
                    removedMentions.current.delete(`true/${p.id}`);
                    onChange({
                      ...draft,
                      mentions: mentions.filter((id) => id !== p.id),
                    });
                  }}
                >
                  <span className="truncate" title={p.name}>
                    @{p.name}
                  </span>
                  <X className="size-3 shrink-0" />
                </Button>
              ))}
          </div>
        )}
        <div
          data-composer-toolbar
          className="flex flex-wrap items-center gap-1 border-t border-border/50 bg-muted/20 px-2 py-1.5 [&>button]:size-11"
        >
          <input
            ref={fileInput}
            aria-label={t("attach")}
            type="file"
            multiple
            className="sr-only"
            disabled={locked || recording}
            onChange={(e) => {
              void attach(Array.from(e.target.files ?? []));
              e.target.value = "";
            }}
          />
          <Button
            size="icon"
            type="button"
            variant="ghost"
            disabled={locked || recording}
            aria-label={t("attach")}
            title={`${t("attach")} · ${t("attachmentHelp")}`}
            onClick={() => fileInput.current?.click()}
          >
            <Paperclip />
          </Button>
          <Button
            size="icon"
            type="button"
            variant="ghost"
            disabled={(locked && !recording) || draft.attachments.length >= 10}
            aria-label={t(recording ? "stopRecording" : "recordVoice")}
            title={t(recording ? "stopRecording" : "recordVoice")}
            onClick={() => void toggleRecording()}
          >
            {recording ? <Square className="text-destructive" /> : <Mic />}
          </Button>
          {group && (
            <Button
              size="icon"
              type="button"
              variant="ghost"
              disabled={locked}
              aria-label={t("mentionAgent")}
              onClick={openMentions}
            >
              <AtSign />
            </Button>
          )}
          <div className="ml-auto flex items-center gap-1">
            {current && (
              <Button
                type="button"
                size="icon"
                variant="outline"
                disabled={busy || current.state === "stopping"}
                aria-label={t("cancel")}
                onClick={onStop}
              >
                <Square className="size-4" />
              </Button>
            )}
            <div className="flex shrink-0 items-stretch">
              <Button
                type="submit"
                size="sm"
                className="rounded-r-none text-xs disabled:opacity-100 disabled:text-primary-foreground/50"
                title={t(mode === "enter" ? "enterSends" : "ctrlEnterSends")}
                disabled={
                  locked ||
                  recording ||
                  (!draft.prompt.trim() && !draft.attachments.length)
                }
              >
                {t(mode === "enter" ? "sendWithEnter" : "sendWithCtrlEnter")}
              </Button>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    type="button"
                    size="sm"
                    className="rounded-l-none border-l border-primary-foreground/25 px-2"
                    aria-label={t("sendShortcut")}
                    title={t("sendShortcut")}
                  >
                    <ChevronDown className="size-3.5" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent side="top" align="end">
                  <DropdownMenuRadioGroup
                    value={mode}
                    onValueChange={(value) => {
                      setMode(value);
                      remember("ui.chat.sendKey", value);
                    }}
                  >
                    <DropdownMenuRadioItem value="enter">
                      {t("enterSends")}
                    </DropdownMenuRadioItem>
                    <DropdownMenuRadioItem value="ctrl-enter">
                      {t("ctrlEnterSends")}
                    </DropdownMenuRadioItem>
                  </DropdownMenuRadioGroup>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          </div>
        </div>
      </div>
      {group && mention && textarea.current && !locked && !recording && (
        <MentionSuggestions
          input={textarea.current}
          token={mention}
          options={mentionOptions}
          active={effectiveMention}
          selected={(a) => (a.human ? mentions : targets).includes(a.id)}
          onActive={setActiveMention}
          onPick={pickAgent}
          onDismiss={dismissMention}
        />
      )}
      {(error || uploading || recording) && (
        <p
          role={error ? "alert" : "status"}
          className={`text-xs ${error ? "text-destructive" : "text-muted-foreground"}`}
        >
          {error || t(recording ? "recording" : "uploading")}
        </p>
      )}
    </AutoForm>
  );
}
