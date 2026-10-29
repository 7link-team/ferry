/**
 * Remote terminal over the host channel. Identical for a runner host and an
 * SSH host — the server hides which one it is, which is the whole point of
 * `HostChannel` (design doc §9.1).
 *
 * Wire format matches `crates/server/src/remote.rs::terminal`:
 *   browser -> server  binary = keystrokes, text = {"type":"resize",cols,rows}
 *   server -> browser  binary = output,     text = {"type":"closed"|"error"}
 */
import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useTheme } from "../theme";

type Status = "connecting" | "open" | "closed" | "error";

/** xterm needs literal colors, so the CSS tokens are read off the element. */
function paletteFrom(element: HTMLElement) {
  const style = getComputedStyle(element);
  const read = (name: string, fallback: string) =>
    style.getPropertyValue(name).trim() || fallback;
  return {
    background: read("--color-card", "#161a21"),
    foreground: read("--color-foreground", "#e6e9ee"),
    cursor: read("--color-foreground", "#e6e9ee"),
    selectionBackground: read("--color-accent", "#3a4250"),
  };
}

export function HostTerminal({ workspace, hostId, directory }: { workspace: string; hostId: string; directory?: string }) {
  const mount = useRef<HTMLDivElement>(null);
  const terminal = useRef<Terminal>(null);
  const [status, setStatus] = useState<Status>("connecting");
  const [error, setError] = useState("");
  const { resolved } = useTheme();

  useEffect(() => {
    const element = mount.current;
    if (!element) return;

    const term = new Terminal({
      fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
      fontSize: 13,
      cursorBlink: true,
      // Enough history to scroll back through a build log without being able
      // to exhaust memory on a chatty process.
      scrollback: 5000,
      theme: paletteFrom(element),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(element);
    fit.fit();
    terminal.current = term;

    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    const socket = new WebSocket(
      `${scheme}//${location.host}/api/workspaces/${encodeURIComponent(workspace)}` +
        `/hosts/${encodeURIComponent(hostId)}/terminal?cols=${term.cols}&rows=${term.rows}`,
    );
    socket.binaryType = "arraybuffer";

    socket.onopen = () => {
      setStatus("open");
      if (directory && !/[\x00-\x1f\x7f]/.test(directory)) socket.send(new TextEncoder().encode(`cd '${directory.replaceAll("'", "'\\''")}'\r`));
    };
    socket.onmessage = (event) => {
      if (typeof event.data === "string") {
        const message = JSON.parse(event.data) as { type: string; error?: string };
        if (message.type === "error") {
          setError(message.error ?? "连接出错。");
          setStatus("error");
        } else {
          setStatus("closed");
        }
        return;
      }
      term.write(new Uint8Array(event.data as ArrayBuffer));
    };
    socket.onerror = () => {
      // The browser never says why a WebSocket failed, so there is nothing
      // truthful to show beyond the fact that it did.
      setStatus((s) => (s === "open" ? "closed" : "error"));
    };
    socket.onclose = () => setStatus((s) => (s === "error" ? s : "closed"));

    const input = term.onData((data) => {
      if (socket.readyState === WebSocket.OPEN) socket.send(new TextEncoder().encode(data));
    });

    const sendSize = () => {
      if (socket.readyState !== WebSocket.OPEN) return;
      socket.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
    };
    const observer = new ResizeObserver(() => {
      // fit() throws if the element is detached mid-animation.
      try {
        fit.fit();
        sendSize();
      } catch {
        /* the pane is being torn down */
      }
    });
    observer.observe(element);

    return () => {
      observer.disconnect();
      input.dispose();
      socket.close();
      term.dispose();
      terminal.current = null;
    };
  }, [workspace, hostId, directory]);

  // Re-theme in place. Recreating the terminal would kill a live session just
  // because someone toggled dark mode.
  useEffect(() => {
    if (terminal.current && mount.current) {
      terminal.current.options.theme = paletteFrom(mount.current);
    }
  }, [resolved]);

  return (
    <div className="flex h-full min-h-[320px] flex-col gap-2">
      <div className="flex items-center gap-2 text-sm">
        <span
          className={
            status === "open"
              ? "text-success"
              : status === "error"
                ? "text-destructive"
                : "text-muted-foreground"
          }
        >
          {status === "connecting" && "连接中…"}
          {status === "open" && "已连接"}
          {status === "closed" && "会话已结束"}
          {status === "error" && (error || "连接失败")}
        </span>
      </div>
      <div
        ref={mount}
        className="min-h-0 flex-1 overflow-hidden rounded-lg border border-border bg-card p-2"
      />
    </div>
  );
}
