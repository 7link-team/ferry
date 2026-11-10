import { useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Bot, Check, UserRound } from "lucide-react";
import { conversationText as t } from "./resource-config";

export interface MentionToken {
  start: number;
  end: number;
  query: string;
}
export interface MentionOption {
  id: string;
  name: string;
  state: string;
  human: boolean;
}

export function mentionRanges(value: string, options: MentionOption[]) {
  const ranges: { start: number; end: number; option: MentionOption }[] = [];
  for (const option of options) {
    const label = `@${option.name}`;
    let from = 0,
      start: number;
    while ((start = value.indexOf(label, from)) !== -1) {
      const end = start + label.length;
      if (
        (start === 0 || /[\s([{]/u.test(value[start - 1]!)) &&
        (end === value.length ||
          /[\s.,!?;:，。！？；：)\]}]/u.test(value[end]!))
      )
        ranges.push({ start, end, option });
      from = end;
    }
  }
  const longest = new Map<number, number>();
  for (const range of ranges)
    longest.set(
      range.start,
      Math.max(longest.get(range.start) ?? 0, range.end),
    );
  return ranges
    .filter((range) => range.end === longest.get(range.start))
    .sort((a, b) => a.start - b.start || b.end - a.end);
}

export function selectMentionDeletion(
  input: HTMLTextAreaElement,
  options: MentionOption[],
  backwards: boolean,
) {
  let start = input.selectionStart,
    end = input.selectionEnd;
  const collapsed = start === end;
  const ranges = mentionRanges(input.value, options);
  const hit = collapsed
    ? ranges.find((range) =>
        backwards
          ? start > range.start &&
            (start <= range.end ||
              (start === range.end + 1 && input.value[range.end] === " "))
          : start >= range.start && start < range.end,
      )
    : undefined;
  if (hit) {
    start = hit.start;
    end = hit.end + (input.value[hit.end] === " " ? 1 : 0);
  } else if (!collapsed) {
    for (const range of ranges) {
      if (start < range.end && end > range.start) {
        start = Math.min(start, range.start);
        end = Math.max(end, range.end);
      }
    }
  }
  if (start !== input.selectionStart || end !== input.selectionEnd)
    input.setSelectionRange(start, end);
}

export function mentionAt(
  value: string,
  start: number,
  end = start,
): MentionToken | null {
  if (start !== end) return null;
  const match = value.slice(0, start).match(/(?:^|[\s([{])@([^\s@]*)$/u);
  if (!match) return null;
  const query = match[1]!;
  return { start: start - query.length - 1, end: start, query };
}

// A typography-matched mirror handles wrapping and textarea scroll without
// moving focus or placing markup in the actual message.
function caretPosition(input: HTMLTextAreaElement, end: number) {
  const style = getComputedStyle(input),
    rect = input.getBoundingClientRect();
  const mirror = document.createElement("div"),
    marker = document.createElement("span");
  for (const property of [
    "box-sizing",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "letter-spacing",
    "line-height",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border-top-width",
    "border-right-width",
    "border-bottom-width",
    "border-left-width",
    "word-spacing",
    "tab-size",
    "text-indent",
    "text-transform",
    "direction",
  ])
    mirror.style.setProperty(property, style.getPropertyValue(property));
  Object.assign(mirror.style, {
    position: "fixed",
    visibility: "hidden",
    pointerEvents: "none",
    margin: "0",
    left: `${rect.left}px`,
    top: `${rect.top}px`,
    width: `${input.clientWidth + (parseFloat(style.borderLeftWidth) || 0) + (parseFloat(style.borderRightWidth) || 0)}px`,
    whiteSpace: "pre-wrap",
    overflowWrap: "break-word",
    borderStyle: "solid",
  });
  mirror.textContent = input.value.slice(0, end);
  marker.textContent = "\u200b";
  mirror.append(marker, document.createTextNode(input.value.slice(end)));
  document.body.append(mirror);
  const caret = marker.getBoundingClientRect();
  const position = {
    x: caret.left - input.scrollLeft,
    y: caret.top - input.scrollTop,
    line: parseFloat(style.lineHeight) || 24,
  };
  mirror.remove();
  position.y = Math.max(
    rect.top,
    Math.min(position.y, rect.bottom - position.line),
  );
  return position;
}

export function MentionSuggestions({
  input,
  token,
  options,
  active,
  selected,
  onActive,
  onPick,
  onDismiss,
}: {
  input: HTMLTextAreaElement;
  token: MentionToken;
  options: MentionOption[];
  active: number;
  selected: (option: MentionOption) => boolean;
  onActive: (index: number) => void;
  onPick: (option: MentionOption) => void;
  onDismiss: () => void;
}) {
  const popup = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{
    left: number;
    top?: number;
    bottom?: number;
    maxHeight: number;
  }>();
  useLayoutEffect(() => {
    function update() {
      const caret = caretPosition(input, token.end),
        viewport = window.visualViewport;
      const top = viewport?.offsetTop ?? 0,
        left = viewport?.offsetLeft ?? 0;
      const width = viewport?.width ?? window.innerWidth,
        height = viewport?.height ?? window.innerHeight;
      const above = caret.y - top - 8,
        below = top + height - caret.y - caret.line - 8;
      const upwards = above >= 180 || above >= below;
      setPosition({
        left: Math.max(
          left + 8,
          Math.min(caret.x, left + width - Math.min(320, width - 16) - 8),
        ),
        ...(upwards
          ? { bottom: window.innerHeight - caret.y + 4 }
          : { top: caret.y + caret.line + 4 }),
        maxHeight: Math.max(44, Math.min(312, (upwards ? above : below) - 8)),
      });
    }
    update();
    const resize = new ResizeObserver(update);
    resize.observe(input);
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    window.visualViewport?.addEventListener("resize", update);
    window.visualViewport?.addEventListener("scroll", update);
    return () => {
      resize.disconnect();
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
      window.visualViewport?.removeEventListener("resize", update);
      window.visualViewport?.removeEventListener("scroll", update);
    };
  }, [input, token]);
  useLayoutEffect(() => {
    popup.current
      ?.querySelector(`[data-index="${active}"]`)
      ?.scrollIntoView?.({ block: "nearest" });
  }, [active]);
  useLayoutEffect(() => {
    const outside = (event: PointerEvent) => {
      if (
        event.target !== input &&
        !popup.current?.contains(event.target as Node)
      )
        onDismiss();
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [input, onDismiss]);
  return createPortal(
    <div
      ref={popup}
      data-mention-popup
      style={{ ...position, visibility: position ? "visible" : "hidden" }}
      className="fixed z-50 flex w-80 max-w-[calc(100vw-1rem)] flex-col overflow-hidden rounded-xl border bg-popover text-popover-foreground shadow-lg"
    >
      <div className="shrink-0 border-b px-3 py-2 text-xs font-medium text-muted-foreground">
        {t("mentionAgent")}
      </div>
      <div
        id="conversation-mention-options"
        role="listbox"
        aria-label={t("mentionAgent")}
        className="min-h-0 overflow-y-auto overscroll-contain p-1"
        style={{ maxHeight: Math.max(44, (position?.maxHeight ?? 312) - 37) }}
      >
        {!options.length && (
          <p role="status" className="px-3 py-3 text-sm text-muted-foreground">
            {t("noMentionResults")}
          </p>
        )}
        {options.map((option, index) => {
          const Icon = option.human ? UserRound : Bot;
          return (
            <button
              key={`${option.human}/${option.id}`}
              id={`conversation-mention-${index}`}
              role="option"
              type="button"
              tabIndex={-1}
              aria-selected={index === active}
              aria-disabled={option.state !== "active"}
              data-index={index}
              onPointerDown={(event) => event.preventDefault()}
              onMouseMove={() => {
                if (option.state === "active") onActive(index);
              }}
              onClick={() => {
                if (option.state === "active") onPick(option);
              }}
              className={`flex min-h-11 w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm ${index === active ? "bg-accent text-accent-foreground" : "hover:bg-accent/60"} ${option.state !== "active" ? "opacity-50" : ""}`}
            >
              <span className="flex size-8 shrink-0 items-center justify-center rounded-md border bg-background">
                <Icon className="size-4" />
              </span>
              <span className="min-w-0 flex-1 truncate" title={option.name}>
                {option.name}
              </span>
              {selected(option) && (
                <Check className="size-4 shrink-0 text-primary" />
              )}
            </button>
          );
        })}
      </div>
    </div>,
    document.body,
  );
}
