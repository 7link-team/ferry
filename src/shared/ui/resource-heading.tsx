import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { api, errorText } from "../api/api";
import { LoadingSkeleton } from "./loading-skeleton";

/** Record identity stays outside tab panels and above section scrolling. */
export function ResourceHeading({ name, children }: { name: string; children?: ReactNode }) {
  const ref = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const node = ref.current;
    const parent = node?.parentElement;
    if (!node || !parent) return;
    const measure = () => parent.style.setProperty("--resource-heading-height", `${node.getBoundingClientRect().height}px`);
    measure();
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure);
    observer?.observe(node);
    return () => { observer?.disconnect(); parent.style.removeProperty("--resource-heading-height"); };
  }, [name]);
  return <header ref={ref} data-resource-heading className="sticky top-0 z-20 min-w-0 shrink-0 border-b bg-background py-3">
    <h2 className="line-clamp-2 break-words text-base font-semibold [overflow-wrap:anywhere]" title={name}>{name}</h2>
    {children}
  </header>;
}

/** Detail shells that otherwise load their record only inside the first tab. */
export function RemoteResourceHeading({ endpoint, fallback }: { endpoint: string; fallback: string }) {
  const [record, setRecord] = useState<{ endpoint: string; name?: string; error?: string }>();
  useEffect(() => {
    const controller = new AbortController();
    void api<{ name?: string; title?: string }>(endpoint, { signal: controller.signal })
      .then(row => { if (!controller.signal.aborted) setRecord({ endpoint, name: row.name || row.title || fallback }); })
      .catch(error => { if (!controller.signal.aborted) setRecord({ endpoint, error: errorText(error) }); });
    return () => controller.abort();
  }, [endpoint, fallback]);
  const current = record?.endpoint === endpoint ? record : undefined;
  if (!current) return <div className="shrink-0 border-b py-3"><LoadingSkeleton variant="inline" /></div>;
  return <ResourceHeading name={current.name || fallback}>{current.error && <p role="alert" className="text-sm text-destructive">{current.error}</p>}</ResourceHeading>;
}
