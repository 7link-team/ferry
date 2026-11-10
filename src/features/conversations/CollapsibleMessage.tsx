import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { Button } from "../../shared/ui/button";
import { conversationText as t } from "./resource-config";

/** Measure rendered content: tables and diagrams can be tall even with few words. */
export function CollapsibleMessage({ children, streaming = false }: { children: ReactNode; streaming?: boolean }) {
  const content = useRef<HTMLDivElement>(null);
  const [long, setLong] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const id = useId();
  useEffect(() => {
    const node = content.current;
    if (!node) return;
    const measure = () => setLong(node.scrollHeight > 480);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  const collapsed = long && !expanded && !streaming;
  return <div className="min-w-0">
    <div id={id} className={collapsed ? "relative max-h-[26rem] overflow-hidden" : "relative"} inert={collapsed ? true : undefined}>
      <div ref={content}>{children}</div>
      {collapsed && <div aria-hidden className="pointer-events-none absolute inset-x-0 bottom-0 h-16 bg-gradient-to-t from-card to-transparent" />}
    </div>
    {long && !streaming && <Button type="button" variant="ghost" className="mt-1 min-h-11 gap-1.5 px-2 text-primary" aria-expanded={expanded} aria-controls={id} onClick={(event) => {
      setExpanded(!expanded);
      if (expanded) event.currentTarget.closest("article")?.scrollIntoView({ block: "start" });
    }}>{expanded ? <ChevronUp className="size-4" /> : <ChevronDown className="size-4" />}{t(expanded ? "collapseMessage" : "expandMessage")}</Button>}
  </div>;
}
