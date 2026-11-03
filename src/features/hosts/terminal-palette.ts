import { useSyncExternalStore } from "react";
import type { ITheme } from "@xterm/xterm";

export const terminalPalettes: Record<string, ITheme> = {
  classic: { background: "#101010", foreground: "#eeeeee", cursor: "#ffffff", selectionBackground: "#505050" },
  light: { background: "#fafafa", foreground: "#202020", cursor: "#202020", selectionBackground: "#b3d4fc" },
  dracula: { background: "#282a36", foreground: "#f8f8f2", cursor: "#f8f8f2", selectionBackground: "#44475a", black: "#21222c", red: "#ff5555", green: "#50fa7b", yellow: "#f1fa8c", blue: "#bd93f9", magenta: "#ff79c6", cyan: "#8be9fd", white: "#f8f8f2" },
  solarized: { background: "#002b36", foreground: "#93a1a1", cursor: "#93a1a1", selectionBackground: "#073642", black: "#073642", red: "#dc322f", green: "#859900", yellow: "#b58900", blue: "#268bd2", magenta: "#d33682", cyan: "#2aa198", white: "#eee8d5" },
};
const key = "ferry.terminal.palette";
const eventName = "ferry:terminal-palette";
let transient = "theme";
function read() { try { const value = localStorage.getItem(key); return value && terminalPalettes[value] ? value : "theme"; } catch { return transient; } }
function subscribe(notify: () => void) {
  window.addEventListener(eventName, notify);
  window.addEventListener("storage", notify);
  return () => { window.removeEventListener(eventName, notify); window.removeEventListener("storage", notify); };
}
export function useTerminalPalette() {
  const palette = useSyncExternalStore(subscribe, read);
  const setPalette = (value: string) => {
    transient = value in terminalPalettes ? value : "theme";
    try { localStorage.setItem(key, transient); } catch { /* Storage may be disabled. */ }
    window.dispatchEvent(new Event(eventName));
  };
  return { palette, setPalette };
}
