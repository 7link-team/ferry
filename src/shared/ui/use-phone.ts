import { useSyncExternalStore } from "react";

const PHONE = "(max-width: 767px)";
/** Phone layout (< 768px, the `md` breakpoint); false where media queries are unavailable. */
export function usePhone(): boolean {
  return useSyncExternalStore(
    notify => {
      if (typeof window.matchMedia !== "function") return () => {};
      const query = window.matchMedia(PHONE);
      query.addEventListener("change", notify);
      return () => query.removeEventListener("change", notify);
    },
    () => typeof window.matchMedia === "function" && window.matchMedia(PHONE).matches,
  );
}
