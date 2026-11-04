import { useEffect, useState } from "react";

/** Finish each request before restarting the countdown; hidden tabs do not poll. */
export function useAutoRefresh(seconds: number, refresh: () => unknown | Promise<unknown>, pending = false) {
  const [remaining, setRemaining] = useState(seconds);
  useEffect(() => {
    let live = true;
    let timer = 0;
    let inFlight = false;
    let deadline = 0;
    setRemaining(pending ? 0 : seconds);
    if (!seconds || pending) return;
    const start = () => {
      if (!live) return;
      deadline = Date.now() + seconds * 1000;
      setRemaining(seconds);
      if (!document.hidden) timer = window.setTimeout(tick, 1000);
    };
    const reload = async () => {
      if (!live || inFlight || document.hidden) return;
      inFlight = true;
      setRemaining(0);
      try { await refresh(); }
      catch { /* The caller reports request errors; polling resumes after failure too. */ }
      finally { inFlight = false; start(); }
    };
    const tick = () => {
      if (!live || document.hidden) return;
      const left = Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
      setRemaining(left);
      if (left === 0) void reload();
      else timer = window.setTimeout(tick, 1000);
    };
    const wake = () => {
      clearTimeout(timer);
      if (!document.hidden) void reload();
    };
    start();
    document.addEventListener("visibilitychange", wake);
    return () => { live = false; clearTimeout(timer); document.removeEventListener("visibilitychange", wake); };
  }, [seconds, refresh, pending]);
  return seconds ? remaining : 0;
}
