import { localeHeaders } from "../i18n/locale";

interface Snapshot {
  accessDeadline: number;
  refreshDeadline: number;
  autoRenew: boolean;
  renewBeforeMs: number;
}
let snapshot: Snapshot | undefined;
let pending: Promise<boolean> | undefined;
let initialIdentity = true;
let resumeOnForeground = false;
let accessRejected = false;
const RENEW_BEFORE_MS = 60_000;

/** Server durations are converted to monotonic deadlines, so a skewed local
 * clock cannot turn the fixed refresh window into a sliding one. */
export function observeSession(value: unknown) {
  initialIdentity = false;
  accessRejected = false;
  snapshot = undefined;
  resumeOnForeground = false;
  if (!value || typeof value !== "object") return;
  const meta = value as Record<string, unknown>;
  const access = meta.access_expires_at_ms;
  const refresh = meta.refresh_expires_at_ms;
  const now = meta.server_time_ms;
  if (
    ![access, refresh, now].every(
      (value) => typeof value === "number" && Number.isSafeInteger(value),
    )
  )
    return;
  const monotonic = performance.now();
  snapshot = {
    accessDeadline: monotonic + (access as number) - (now as number),
    refreshDeadline: monotonic + (refresh as number) - (now as number),
    autoRenew: meta.auto_renew === true && meta.refreshable === true,
    renewBeforeMs:
      typeof meta.access_ttl_ms === "number" &&
      Number.isSafeInteger(meta.access_ttl_ms) &&
      meta.access_ttl_ms > 0
        ? Math.min(RENEW_BEFORE_MS, meta.access_ttl_ms / 5)
        : RENEW_BEFORE_MS,
  };
  initialIdentity = false;
}

export function clearSession() {
  snapshot = undefined;
  initialIdentity = false;
  resumeOnForeground = false;
  accessRejected = false;
}

export function canResumeSession() {
  return (
    (!!snapshot?.autoRenew && performance.now() < snapshot.refreshDeadline) ||
    resumeOnForeground
  );
}

export function deferUnauthorized() {
  if (!canResumeSession()) return false;
  // Another tab may already have rotated the access token. Check the browser's
  // current cookies on the next real action, even before our old local deadline.
  accessRejected = true;
  return true;
}

export async function waitForSessionRenewal(path: string) {
  if (path !== "/sessions/refresh") await pending?.catch(() => false);
}

async function rotate() {
  // A second tab acquiring the lock sees the first tab's cookies. Reuse that
  // success instead of rotating the same stale refresh credential again.
  const identity = await fetch("/api/me", {
    credentials: "same-origin",
    headers: localeHeaders({ "X-Ferry-Request": "1" }),
  });
  if (identity.ok) {
    const data = await identity.json();
    observeSession(data.session);
    if (
      !snapshot?.autoRenew ||
      snapshot.accessDeadline >= snapshot.refreshDeadline ||
      snapshot.accessDeadline - performance.now() > snapshot.renewBeforeMs
    ) {
      window.dispatchEvent(new Event("session:renewed"));
      return true;
    }
  } else if (identity.status !== 401) {
    throw new Error("Session identity request failed");
  }
  const response = await fetch("/api/sessions/refresh", {
    method: "POST",
    credentials: "same-origin",
    headers: localeHeaders({ "X-Ferry-Request": "1" }),
  });
  if (response.ok) {
    observeSession(await response.json());
    resumeOnForeground = false;
    window.dispatchEvent(new Event("session:renewed"));
    return true;
  }
  if (response.status === 403) {
    if (snapshot) snapshot.autoRenew = false;
    resumeOnForeground = false;
    return false;
  }
  if (response.status === 401) {
    clearSession();
    return false;
  }
  throw new Error("Session renewal request failed");
}

function renew(): Promise<boolean> {
  if (!pending) {
    const operation = async () => {
      if (navigator.locks?.request)
        return await navigator.locks.request("session-renewal", rotate);
      return await rotate();
    };
    pending = operation().finally(() => {
      pending = undefined;
    });
  }
  return pending;
}

/** Only the first foreground identity load may restore an expired access
 * cookie. Later background reads never initiate a refresh. */
export async function restoreInitialSession(path: string) {
  if (path !== "/me" || !initialIdentity) return false;
  initialIdentity = false;
  if (document.visibilityState === "hidden") {
    resumeOnForeground = true;
    return false;
  }
  return renew().catch(() => false);
}

/** No interval: polling, logs, downloads and programmatic scrolls are not
 * evidence of a person using the application. */
export function watchSessionActivity() {
  const activity = () => {
    if (document.visibilityState === "hidden") return;
    // A final clamped grant already lasts until the absolute deadline.
    if (!accessRejected && snapshot && snapshot.accessDeadline >= snapshot.refreshDeadline) return;
    if (
      !resumeOnForeground &&
      (!canResumeSession() ||
        !snapshot ||
        (!accessRejected &&
          snapshot.accessDeadline - performance.now() > snapshot.renewBeforeMs))
    )
      return;
    const expired = !snapshot || snapshot.accessDeadline <= performance.now();
    void renew()
      .then((ok) => {
        if (!ok && (expired || !snapshot) && !canResumeSession())
          window.dispatchEvent(new Event("ferry:unauthorized"));
      })
      .catch(() => {
        /* Preserve the page and draft during a network failure. */
      });
  };
  const events = ["pointerdown", "keydown", "touchstart", "wheel"] as const;
  for (const event of events)
    window.addEventListener(event, activity, { capture: true, passive: true });
  window.addEventListener("focus", activity);
  document.addEventListener("visibilitychange", activity);
  return () => {
    for (const event of events)
      window.removeEventListener(event, activity, true);
    window.removeEventListener("focus", activity);
    document.removeEventListener("visibilitychange", activity);
  };
}
