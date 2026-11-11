import { CircleHelp } from "lucide-react";
import { Popover } from "radix-ui";
import { Children, createContext, isValidElement, useContext, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { commonText } from "../i18n/i18n";

export const FieldHelpContext = createContext<(label: ReactNode) => ReactNode>(() => null);
export function fieldLabelText(node: ReactNode): string {
  return Children.toArray(node).map(child => isValidElement<{ children?: ReactNode }>(child) ? fieldLabelText(child.props.children) : typeof child === "string" || typeof child === "number" ? String(child) : "").join("").trim();
}

/** A non-labelable trigger also works inside existing native field labels. */
export function FieldHelp({ label, children }: { label?: ReactNode; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const enter = () => { clearTimeout(timer.current); setOpen(true); };
  const leave = () => { clearTimeout(timer.current); timer.current = setTimeout(() => setOpen(false), 180); };
  useEffect(() => () => clearTimeout(timer.current), []);
  return <Popover.Root open={open} onOpenChange={setOpen}>
    <Popover.Trigger asChild>
      <span role="button" tabIndex={0} aria-label={`${fieldLabelText(label)} · ${commonText("field.help")}`} className="ml-1 inline-flex size-6 shrink-0 cursor-help items-center justify-center rounded text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" onPointerEnter={enter} onPointerLeave={leave} onFocus={enter} onBlur={leave} onClick={event => { event.preventDefault(); event.stopPropagation(); enter(); }} onKeyDown={event => {
        if (event.key === "Enter" || event.key === " ") { event.preventDefault(); event.stopPropagation(); enter(); }
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setOpen(false); }
      }}><CircleHelp aria-hidden="true" className="size-3.5" /></span>
    </Popover.Trigger>
    <Popover.Portal><Popover.Content role="tooltip" side="top" align="start" sideOffset={6} collisionPadding={12} className="z-[100] max-h-[50dvh] w-80 max-w-[calc(100vw-1.5rem)] overflow-auto whitespace-pre-wrap rounded-md border bg-popover p-3 text-sm font-normal leading-6 text-popover-foreground shadow-md" onOpenAutoFocus={event => event.preventDefault()} onCloseAutoFocus={event => event.preventDefault()} onPointerEnter={enter} onPointerLeave={leave}>
      {children}
    </Popover.Content></Popover.Portal>
  </Popover.Root>;
}

export function FieldCaption({ label, help, required = false }: { label: ReactNode; help?: ReactNode; required?: boolean }) {
  const resolve = useContext(FieldHelpContext);
  const id = useId();
  const frame = useRef<HTMLSpanElement>(null);
  const [hint, setHint] = useState("");
  useEffect(() => {
    const container = frame.current?.closest("[data-auto-field]") ?? frame.current?.closest("label");
    const control = container?.querySelector<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>("input:not([aria-hidden=true]),select:not([aria-hidden=true]),textarea,[role=switch],[role=checkbox]");
    if (!control) { setHint(""); return; }
    // The help trigger has its own accessible name; exclude it from the input's name.
    if (!control.hasAttribute("aria-label") && (!control.hasAttribute("aria-labelledby") || control.dataset.fieldCaption === "true")) {
      control.setAttribute("aria-labelledby", id);
      control.dataset.fieldCaption = "true";
    }
    if (control.disabled || control.hasAttribute("readonly") || control.getAttribute("aria-readonly") === "true") {
      setHint("");
      return;
    }
    const type = control.tagName === "SELECT" || control.getAttribute("role") === "combobox" ? "select" : control.tagName === "TEXTAREA" ? "multiline" : control.type === "checkbox" || ["switch", "checkbox"].includes(control.getAttribute("role") ?? "") ? "toggle" : control.type === "number" ? "number" : "text";
    const example = control.getAttribute("placeholder");
    const limits = [control.getAttribute("min"), control.getAttribute("max")].filter(value => value !== null).join(" … ");
    const next = commonText(`field.${type}`) + (limits ? ` (${limits})` : "") + (example ? "\n" + commonText("field.example") + example : "") + (control.required ? "\n" + commonText("field.required") : "");
    setHint(current => current === next ? current : next);
  });
  const content = help ?? resolve(label) ?? hint;
  return <span ref={frame} className="inline-flex max-w-full items-center"><span id={id} data-field-caption-text>{label}</span>{required && <span className="ml-1 text-destructive" aria-hidden="true">*</span>}{content && <FieldHelp label={label}>{content}</FieldHelp>}</span>;
}
