import { useCallback,useEffect,useState } from "react";
import { CUSTOM_VARIABLE_KEYS,sanitizeCustomThemeVariables,type ThemeVariables } from "./theme-validation";

export const PALETTES = [
  { value: "modern", label: "现代简约" },
  { value: "business", label: "商务渐变" },
  { value: "eyeCare", label: "护眼模式" },
  { value: "pinkKawaii", label: "粉色可爱" },
  { value: "neoBrutalism", label: "新粗野主义" },
  { value: "custom", label: "自定义" },
] as const;
export type Palette = (typeof PALETTES)[number]["value"];
export type Mode = "light" | "dark" | "system";
type ThemeState = { palette: Palette; mode: Mode; customLight: ThemeVariables; customDark: ThemeVariables };
const KEY = "ferry.theme";
const DEFAULT: ThemeState = { palette: "modern", mode: "system", customLight: {}, customDark: {} };
let current: ThemeState | undefined;

function read(): ThemeState {
  if (current) return current;
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? "null") as Partial<ThemeState> | null;
    return {
      palette: PALETTES.some(p => p.value === saved?.palette) ? saved!.palette! : DEFAULT.palette,
      mode: ["light", "dark", "system"].includes(saved?.mode ?? "") ? saved!.mode! : DEFAULT.mode,
      customLight: sanitizeCustomThemeVariables(saved?.customLight),
      customDark: sanitizeCustomThemeVariables(saved?.customDark),
    };
  } catch { return { ...DEFAULT }; }
}
const prefersDark = () => window.matchMedia("(prefers-color-scheme: dark)").matches;

/** Apply before React mounts, including custom colors, to avoid a first-paint flash. */
export function applyTheme(palette: Palette, mode: Mode) {
  const root = document.documentElement;
  const dark = mode === "dark" || (mode === "system" && prefersDark());
  root.classList.toggle("dark", dark);
  root.removeAttribute("data-palette");
  root.setAttribute("data-theme", palette);
  root.style.colorScheme = dark ? "dark" : "light";
  CUSTOM_VARIABLE_KEYS.forEach(key => root.style.removeProperty(key));
  if (palette === "custom") {
    Object.entries(dark ? read().customDark : read().customLight).forEach(([key, value]) => root.style.setProperty(key, value));
  }
}
export function initTheme() {
  current = undefined;
  const state = read();
  applyTheme(state.palette, state.mode);
}
const listeners = new Set<() => void>();
function update(next: Partial<ThemeState>) {
  current = { ...read(), ...next };
  try { localStorage.setItem(KEY, JSON.stringify(current)); } catch { /* Keep the in-memory preference when storage is unavailable. */ }
  applyTheme(current.palette, current.mode);
  listeners.forEach(fn => fn());
}
export function useTheme() {
  const [state, setState] = useState(read);
  useEffect(() => {
    const sync = () => setState({ ...read() });
    const mediaSync = () => { const value = read(); applyTheme(value.palette, value.mode); sync(); };
    const storageSync = (event: StorageEvent) => { if (event.key === KEY || event.key === null) { current = undefined; mediaSync(); } };
    listeners.add(sync);
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    media.addEventListener("change", mediaSync);
    window.addEventListener("storage", storageSync);
    return () => { listeners.delete(sync); media.removeEventListener("change", mediaSync); window.removeEventListener("storage", storageSync); };
  }, []);
  const set = useCallback((next: Partial<Pick<ThemeState, "palette" | "mode">>) => update(next), []);
  const setCustomVariable = useCallback((mode: "light" | "dark", key: string, value: string) => {
    const name = mode === "dark" ? "customDark" : "customLight";
    const variables = { ...read()[name] };
    if (!value.trim()) delete variables[key];
    else {
      const sanitized = sanitizeCustomThemeVariables({ [key]: value.trim() });
      if (!sanitized[key]) return;
      variables[key] = sanitized[key];
    }
    update({ [name]: variables });
  }, []);
  const resetCustomVariables = useCallback((mode?: "light" | "dark") => update(mode ? { [mode === "dark" ? "customDark" : "customLight"]: {} } : { customLight: {}, customDark: {} }), []);
  const resolved: "light" | "dark" = state.mode === "system" ? (prefersDark() ? "dark" : "light") : state.mode;
  return { ...state, resolved, set, setCustomVariable, resetCustomVariables };
}
