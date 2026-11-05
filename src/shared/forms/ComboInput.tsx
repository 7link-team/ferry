import { ChevronDown } from "lucide-react";
import { useId, useState } from "react";
import { Input } from "../ui/input";
import { commonText } from "../i18n/i18n";

/** Searchable suggestions while preserving custom values and expressions verbatim. */
export function ComboInput({ options, onValueChange, ...props }: Omit<React.ComponentProps<typeof Input>, "onChange"> & { options: readonly string[]; onValueChange: (value: string) => void }) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const matches = options.filter(option => option.toLowerCase().includes(query.toLowerCase()));
  const select = (value: string) => { onValueChange(value); setOpen(false); };
  return <div className="relative min-w-0 flex-1" onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setOpen(false); }}>
    <Input {...props} className={`pr-9 ${props.className ?? ""}`} role="combobox" aria-autocomplete="list" aria-expanded={open} aria-controls={id} aria-activedescendant={open && matches.length ? `${id}-${Math.min(active, matches.length - 1)}` : undefined} onFocus={event => { if (!props.readOnly && !props.disabled) { setQuery(""); setActive(0); setOpen(true); } props.onFocus?.(event); }} onChange={event => { onValueChange(event.target.value); setQuery(event.target.value); setActive(0); setOpen(true); }} onKeyDown={event => {
      if (event.key === "Escape" && open) { event.preventDefault(); event.stopPropagation(); setOpen(false); }
      else if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); setOpen(true); setActive(current => Math.max(0, Math.min(matches.length - 1, current + (event.key === "ArrowDown" ? 1 : -1)))); }
      else if (event.key === "Enter" && open) { event.preventDefault(); if (matches[active]) select(matches[active]); }
      props.onKeyDown?.(event);
    }} />
    {!props.readOnly && !props.disabled && <button type="button" aria-label={`${commonText("common.select")} · ${props["aria-label"] ?? ""}`} aria-expanded={open} tabIndex={-1} className="absolute right-0 top-0 flex size-9 items-center justify-center rounded-r-md text-muted-foreground" onMouseDown={event => event.preventDefault()} onClick={() => { setQuery(""); setOpen(current => !current); }}><ChevronDown className="size-4" /></button>}
    {open && matches.length > 0 && <div role="listbox" id={id} className="absolute inset-x-0 top-full z-40 mt-1 max-h-56 overflow-auto rounded-md border bg-popover p-1 text-popover-foreground shadow-md">{matches.map((option, index) => <button type="button" key={option} role="option" id={`${id}-${index}`} aria-selected={String(props.value) === option} tabIndex={-1} className={`block min-h-9 w-full break-all rounded px-2 py-1 text-left text-sm hover:bg-accent ${index === active ? "bg-accent" : ""}`} onMouseDown={event => event.preventDefault()} onMouseEnter={() => setActive(index)} onClick={() => select(option)}>{option}</button>)}</div>}
  </div>;
}
