/**
 * Theme = a palette plus a light/dark mode, both on <html>:
 *   <html class="dark" data-palette="erp">
 *
 * The palettes themselves are value blocks in style.css; nothing here knows
 * their colors. Adding one means adding two CSS blocks and one PALETTES entry.
 *
 * ponytail: localStorage, not a server setting. It is a per-viewer preference
 * on one device — it never needs to reach another browser or Claude. Reads are
 * guarded because a private window can throw instead of returning null.
 */
import { useCallback, useEffect, useState } from "react";

export const PALETTES = [
  { value: "shadcn", label: "默认" },
  { value: "erp", label: "ERP" },
] as const;

export type Palette = (typeof PALETTES)[number]["value"];
export type Mode = "light" | "dark" | "system";

const KEY = "ferry.theme";
const DEFAULT: { palette: Palette; mode: Mode } = { palette: "shadcn", mode: "system" };

function read() {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return DEFAULT;
    const saved = JSON.parse(raw) as Partial<typeof DEFAULT>;
    return {
      palette: PALETTES.some((p) => p.value === saved.palette) ? saved.palette! : DEFAULT.palette,
      mode: ["light", "dark", "system"].includes(saved.mode ?? "") ? saved.mode! : DEFAULT.mode,
    };
  } catch {
    return DEFAULT;
  }
}

const prefersDark = () => window.matchMedia("(prefers-color-scheme: dark)").matches;

/** Writes the attributes. Called on load before React mounts, and on change. */
export function applyTheme(palette: Palette, mode: Mode) {
  const root = document.documentElement;
  const dark = mode === "dark" || (mode === "system" && prefersDark());
  root.classList.toggle("dark", dark);
  // The default palette is the bare :root block, so it carries no attribute.
  if (palette === "shadcn") root.removeAttribute("data-palette");
  else root.setAttribute("data-palette", palette);
  root.style.colorScheme = dark ? "dark" : "light";
}

/** Runs once at startup so the first paint is already the right theme. */
export function initTheme() {
  const { palette, mode } = read();
  applyTheme(palette, mode);
}

const listeners = new Set<() => void>();

export function useTheme() {
  const [state, setState] = useState(read);
  useEffect(() => {
    const sync = () => setState(read());
    listeners.add(sync);
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", sync);
    return () => {
      listeners.delete(sync);
      media.removeEventListener("change", sync);
    };
  }, []);

  const set = useCallback((next: Partial<{ palette: Palette; mode: Mode }>) => {
    const merged = { ...read(), ...next };
    try {
      localStorage.setItem(KEY, JSON.stringify(merged));
    } catch {
      /* a private window refuses to store; the attributes still apply */
    }
    applyTheme(merged.palette, merged.mode);
    listeners.forEach((fn) => fn());
  }, []);

  const resolved: "light" | "dark" =
    state.mode === "system" ? (prefersDark() ? "dark" : "light") : state.mode;

  return { ...state, resolved, set };
}
