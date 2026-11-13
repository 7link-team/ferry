import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { commonText } from "../i18n/i18n";
import { ResourceHeading } from "./resource-heading";

type Section = { key: string; label: string; content: ReactNode };

export function ConfigurationSection({ title, actions, children }: { title: string; actions?: ReactNode; children: ReactNode }) {
  return <section aria-label={title} data-configuration-section className="min-w-0 w-full space-y-4 rounded-xl border bg-card p-4 text-card-foreground sm:p-5">
    <div className="flex min-w-0 flex-wrap items-start justify-between gap-3">
      <h2 className="min-w-0 break-words text-sm font-semibold">{title}</h2>
      {actions && <div className="flex min-w-0 flex-wrap items-center gap-2">{actions}</div>}
    </div>
    {children}
  </section>;
}

/** A table of contents for one continuous form, using its existing scroll container. */
export function ScrollFormSections({ name, sections, footer }: { name: string; sections: Section[]; footer: ReactNode }) {
  const root = useRef<HTMLDivElement>(null);
  const prefix = useId();
  const [active, setActive] = useState(() => new URLSearchParams(location.search).get("section") ?? sections[0]?.key ?? "");
  const scrollContainer = () => {
    let node: HTMLElement | null = root.current;
    while (node && !/(auto|scroll)/.test(getComputedStyle(node).overflowY)) node = node.parentElement;
    return node;
  };
  const elements = () => Array.from(root.current?.querySelectorAll<HTMLElement>("[data-form-section]") ?? []);
  const offset = () => {
    const node = root.current;
    if (!node) return 0;
    const heading = node.querySelector<HTMLElement>("[data-resource-heading]");
    const nav = node.querySelector<HTMLElement>("[data-section-navigation]");
    return (heading?.getBoundingClientRect().height ?? 0) + (matchMedia("(min-width: 768px)").matches ? 0 : nav?.getBoundingClientRect().height ?? 0) + 12;
  };
  const record = (key: string, top: number) => {
    setActive(current => current === key ? current : key);
    const params = new URLSearchParams(location.search);
    params.set("section", key);
    if (top > 8) params.set("at", String(Math.round(top))); else params.delete("at");
    // Like useScrollSync, scrolling replaces the URL without starting route loading.
    history.replaceState(history.state, "", `${location.pathname}?${params}`);
  };
  const go = (key: string) => {
    const node = scrollContainer();
    const target = elements().find(element => element.dataset.formSection === key);
    if (!node || !target) return;
    node.scrollTop += target.getBoundingClientRect().top - node.getBoundingClientRect().top - offset();
    record(key, node.scrollTop);
  };
  // Keys remain stable while fields change. Rebinding to draft objects would jump on every keystroke.
  const keys = sections.map(section => section.key).join("\n");
  useEffect(() => {
    const node = scrollContainer();
    if (!node) return;
    let frame = 0;
    const inspect = () => {
      const line = node.getBoundingClientRect().top + offset();
      const visible = elements();
      const atEnd = node.scrollHeight > node.clientHeight && node.scrollTop + node.clientHeight >= node.scrollHeight - 2;
      const current = atEnd ? visible.at(-1) : visible.filter(element => element.getBoundingClientRect().top <= line + 2).at(-1) ?? visible[0];
      if (current?.dataset.formSection) record(current.dataset.formSection, node.scrollTop);
    };
    const onScroll = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(inspect); };
    const restore = () => {
      const params = new URLSearchParams(location.search);
      const key = params.get("section");
      if (key && !params.has("at")) go(key);
      inspect();
    };
    // The owning form restores ?at= in its effect before this frame.
    frame = requestAnimationFrame(restore);
    node.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", onScroll);
    window.addEventListener("popstate", restore);
    return () => { cancelAnimationFrame(frame); node.removeEventListener("scroll", onScroll); window.removeEventListener("resize", onScroll); window.removeEventListener("popstate", restore); };
  }, [keys]);
  return <div ref={root} className="relative min-w-0" data-scroll-form-sections>
    <ResourceHeading name={name} />
    <div className="flex min-w-0 flex-col gap-4 md:flex-row">
      <nav data-section-navigation aria-label={commonText("form.categories")} className="sticky top-[var(--resource-heading-height,0px)] z-10 flex shrink-0 flex-wrap gap-1 border-b bg-background py-2 md:max-h-[65dvh] md:w-40 md:flex-col md:flex-nowrap md:self-start md:overflow-y-auto md:border-b-0">
        {sections.map(section => {
          const params = new URLSearchParams(location.search); params.set("section", section.key); params.delete("at");
          return <a key={section.key} href={`${location.pathname}?${params}`} aria-current={active === section.key ? "location" : undefined} aria-controls={`${prefix}-${section.key}`} onClick={event => {
            if (event.button || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
            event.preventDefault(); go(section.key);
          }} className={`flex min-h-11 items-center rounded-md px-3 py-2 text-sm break-words focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${active === section.key ? "bg-accent font-medium text-accent-foreground" : "text-foreground hover:bg-accent"}`}>{section.label}</a>;
        })}
      </nav>
      <div className="min-w-0 flex-1 space-y-5 pt-3" data-form-content>
        {sections.map(section => <div key={section.key} id={`${prefix}-${section.key}`} data-form-section={section.key} className="min-w-0 space-y-4">{section.content}</div>)}
        {footer}
      </div>
    </div>
  </div>;
}
