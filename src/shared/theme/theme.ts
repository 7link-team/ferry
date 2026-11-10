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
export type ThemeState = { palette: Palette; mode: Mode; customLight: ThemeVariables; customDark: ThemeVariables };
export const THEME_PREFERENCE_EVENT = "ui:theme-preference";
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

/** Canvas resolves modern CSS colors to the literal sRGB values required by renderers. */
export function createThemeColorReader(element: Element) {
  const style = getComputedStyle(element);
  const canvas = document.createElement("canvas"); canvas.width = canvas.height = 1;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  return (name: string, fallback: string) => {
    const value = style.getPropertyValue(name).trim() || fallback;
    if (!context) return fallback;
    context.clearRect(0, 0, 1, 1); context.fillStyle = fallback; context.fillStyle = value; context.fillRect(0, 0, 1, 1);
    const [r, g, b] = context.getImageData(0, 0, 1, 1).data;
    return `#${[r, g, b].map(channel => channel!.toString(16).padStart(2, "0")).join("")}`;
  };
}

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
function update(next: Partial<ThemeState>, persist = true) {
  current = { ...read(), ...next };
  try { localStorage.setItem(KEY, JSON.stringify(current)); } catch { /* Keep the in-memory preference when storage is unavailable. */ }
  applyTheme(current.palette, current.mode);
  listeners.forEach(fn => fn());
  if (persist) window.dispatchEvent(new CustomEvent(THEME_PREFERENCE_EVENT, { detail: { ...current } }));
}
/** Cloud data passes through the same allowlists as browser preferences. */
export function restoreTheme(value?: Partial<ThemeState> | null) {
  update({
    palette: PALETTES.some(p => p.value === value?.palette) ? value!.palette! : DEFAULT.palette,
    mode: ["light", "dark", "system"].includes(value?.mode ?? "") ? value!.mode! : DEFAULT.mode,
    customLight: sanitizeCustomThemeVariables(value?.customLight),
    customDark: sanitizeCustomThemeVariables(value?.customDark),
  }, false);
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
