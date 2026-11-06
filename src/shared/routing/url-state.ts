import { useCallback,useEffect,useRef,useSyncExternalStore } from "react";
import { LOCALE_EVENT } from "../i18n/locale";
import { beginRoute } from "../api/progress";

const listeners = new Set<() => void>();
export const announce = () => {
  beginRoute(snapshot());
  listeners.forEach((fn) => fn());
};
export const subscribe = (fn: () => void) => {
  if (!listeners.size) {
    window.addEventListener("popstate", announce);
    window.addEventListener(LOCALE_EVENT, announce);
  }
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
    if (!listeners.size) {
      window.removeEventListener("popstate", announce);
      window.removeEventListener(LOCALE_EVENT, announce);
    }
  };
};
export const snapshot = () => location.pathname + location.search;

/** `useQueryValue("page", 1)` must give back a `number`, not the literal `1`. */
type Widen<T> = T extends number ? number : T extends string ? string : T;

/** A filter or pagination value that lives in the URL without adding history. */
export function useQueryValue<T extends string | number>(
  key: string,
  fallback: T,
): readonly [Widen<T>, (value: Widen<T>) => void] {
  useSyncExternalStore(subscribe, snapshot);
  const raw = new URLSearchParams(location.search).get(key);
  const value = (raw === null ? fallback : typeof fallback === "number"
    ? /^\d+$/.test(raw) && Number(raw) > 0 ? Number(raw) : fallback : raw) as Widen<T>;
  const setValue = useCallback((next: Widen<T>) => {
    const params = new URLSearchParams(location.search);
    if (next === fallback) params.delete(key); else params.set(key, String(next));
    if (next !== value && /(^|\.)(page|page_size|sort|folder|tab|section|form_tab|pattern)$/.test(key)) params.delete("at");
    const search = params.toString();
    const target = `${location.pathname}${search ? `?${search}` : ""}`;
    if (target !== snapshot()) { history.replaceState(history.state, "", target); announce(); }
  }, [key, fallback, value]);
  return [value, setValue] as const;
}

/**
 * Keeps a scroll container's offset in `?at=` so a shared link lands where the
 * sender was looking. Restoration waits for `ready` — the rows have to exist
 * before the offset means anything.
 *
 * ponytail: pixels, not a row key. Pixels are wrong if the viewer's page size
 * or font differs; switch to the first visible row's id if that shows up.
 */
export function useScrollSync<T extends HTMLElement = HTMLDivElement>(ready: boolean) {
  const ref = useRef<T>(null);
  const scrollNode = () => {
    let node: HTMLElement | null = ref.current;
    while (node && !/(auto|scroll)/.test(getComputedStyle(node).overflowY)) node = node.parentElement;
    return node ?? ref.current;
  };
  const url = useSyncExternalStore(subscribe, snapshot);
  const path = location.pathname;
  const at = Math.max(0, Math.min(100_000_000, Number(new URLSearchParams(location.search).get("at")) || 0));
  const restored = useRef("");
  useEffect(() => {
    const node = scrollNode();
    if (!node) return;
    let timer = 0;
    const save = () => {
      clearTimeout(timer);
      if (location.pathname !== path) return;
      const params = new URLSearchParams(location.search);
      if (node.scrollTop > 8) params.set("at", String(Math.round(node.scrollTop))); else params.delete("at");
      const search = params.toString();
      history.replaceState(history.state, "", `${path}${search ? `?${search}` : ""}`);
    };
    const onScroll = () => { clearTimeout(timer); timer = window.setTimeout(save, 150); };
    node.addEventListener("scroll", onScroll, {passive:true});
    window.addEventListener("ferry:before-navigate", save);
    return () => { clearTimeout(timer); node.removeEventListener("scroll", onScroll); window.removeEventListener("ferry:before-navigate", save); };
  }, [path, ready]);
  useEffect(() => {
    const node = scrollNode();
    const key = `${path}:${at}`;
    if (!node || !ready || restored.current === key) return;
    restored.current = key;
    node.scrollTop = at;
  }, [ready, path, at, url]);
  return ref;
}
