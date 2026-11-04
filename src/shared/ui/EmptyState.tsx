import type { ReactNode } from "react";

/** Decorative, theme-aware empty states shared by every AutoTable. */
export function EmptyState({ children, filtered = false }: { children: ReactNode; filtered?: boolean }) {
  return <div className="mx-auto flex max-w-md flex-col items-center gap-3" role="status" data-empty-state={filtered ? "filtered" : "empty"}>
    <svg aria-hidden="true" width="104" height="72" viewBox="0 0 104 72" fill="none" className="text-primary">
      <ellipse cx="52" cy="64" rx="38" ry="4" fill="currentColor" opacity=".06" />
      <rect x="21" y="12" width="62" height="46" rx="6" fill="currentColor" opacity=".05" />
      <rect x="21" y="12" width="62" height="46" rx="6" stroke="currentColor" strokeOpacity=".22" />
      <path d="M22 25h60M31 34h21M31 43h16" stroke="currentColor" strokeOpacity=".3" strokeWidth="2" strokeLinecap="round" />
      {filtered ? <g stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><circle cx="68" cy="43" r="10" className="fill-background" /><path d="m75 50 9 9M64 43h8" /></g>
        : <g stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M60 42h16M68 34v16" opacity=".6" /></g>}
      <circle cx="14" cy="31" r="2" fill="currentColor" opacity=".2" />
      <circle cx="90" cy="17" r="3" fill="currentColor" opacity=".12" />
    </svg>
    <div className="text-sm leading-relaxed text-muted-foreground">{children}</div>
  </div>;
}
