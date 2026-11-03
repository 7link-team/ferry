import { RefreshCw } from "lucide-react";
import type { ReactNode } from "react";

/** The refresh action shares the selector's frame without nesting interactive controls. */
export function SelectRefresh({ children, label, loading = false, disabled = false, onRefresh, className = "" }: {
  children: ReactNode;
  label: string;
  loading?: boolean;
  disabled?: boolean;
  onRefresh: () => void;
  className?: string;
}) {
  return <div data-slot="select-refresh" className={`relative min-w-0 [&>select]:pl-10 [&>input]:pl-10 [&>[role=combobox]]:pl-10 [&>button[aria-haspopup]]:pl-10 ${className}`}>
    {children}
    <button type="button" aria-label={label} title={label} disabled={disabled || loading} aria-busy={loading}
      className="absolute inset-y-1 left-1 z-10 flex w-7 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
      onPointerDown={event => event.stopPropagation()} onClick={event => { event.preventDefault(); event.stopPropagation(); onRefresh(); }}>
      <RefreshCw aria-hidden="true" className={`size-3.5 ${loading ? "animate-spin" : ""}`} />
    </button>
  </div>;
}
