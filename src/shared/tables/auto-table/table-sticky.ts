import type { CSSProperties } from "react";

/**
 * Shared sticky-table configuration + cell-style computation used by AutoTable. Left/right
 * columns are pinned horizontally, the header row vertically, and the
 * pagination bar to the bottom. Sticky columns need a width (left/right offsets are summed
 * from column widths); a fallback is used when a width is missing.
 */
export interface StickyConfig {
  /** Pin the header row to the top (requires `maxHeight` for a vertical scroll). */
  header?: boolean;
  /** Number of leftmost columns to pin. */
  left?: number;
  /** Number of rightmost columns to pin (the actions column counts as the last column). */
  right?: number;
  /** Pin the pagination bar to the bottom of the table block. */
  pagination?: boolean;
  /** Scroll-container max height (e.g. '70vh', 480) — enables vertical scrolling + sticky header. */
  maxHeight?: string | number;
}

export function baseTableStickyClassName(
  cfg: StickyConfig | undefined,
): string | undefined {
  if (!cfg) return undefined;
  return [
    "border-separate border-spacing-0",
    cfg.left || cfg.right ? "min-w-max" : "",
    cfg.header
      ? "[&_thead]:sticky [&_thead]:top-0 [&_thead]:z-20 [&_thead]:bg-background [&_thead]:shadow-[0_4px_8px_-6px_var(--sticky-shadow-color)]"
      : "",
    (cfg.left ?? 0) > 0
      ? "[&_tr>*:first-child]:sticky [&_tr>*:first-child]:left-0 [&_tr>*:first-child]:z-10 [&_tr>*:first-child]:min-w-[150px] [&_tr>*:first-child]:bg-background [&_tr>*:first-child]:shadow-[4px_0_8px_-6px_var(--sticky-shadow-color)]"
      : "",
    (cfg.right ?? 0) > 0
      ? "[&_tr>*:last-child]:sticky [&_tr>*:last-child]:right-0 [&_tr>*:last-child]:z-10 [&_tr>*:last-child]:min-w-[120px] [&_tr>*:last-child]:bg-background [&_tr>*:last-child]:shadow-[-4px_0_8px_-6px_var(--sticky-shadow-color)]"
      : "",
  ]
    .filter(Boolean)
    .join(" ");
}

const FALLBACK_WIDTH = 150;

export function stickyColWidth(width?: string | number): number {
  if (typeof width === "number") return width;
  if (typeof width === "string") {
    const n = parseFloat(width);
    if (Number.isFinite(n)) return n;
  }
  return FALLBACK_WIDTH;
}

// z-index layers: body sticky col < header row < header sticky col (corner)
const Z_COL = 10;
const Z_HEADER = 20;
const Z_CORNER = 30;

/**
 * Computes the sticky style for a cell at `index` of `total` columns.
 * `widths` is the resolved px width per column. `isHeader` raises the z-index so the header
 * sits above sticky body columns and corner cells above both.
 */
export function stickyCellStyle(
  index: number,
  total: number,
  widths: number[],
  cfg: StickyConfig | undefined,
  isHeader = false,
): CSSProperties | undefined {
  if (!cfg) return undefined;
  const left = cfg.left ?? 0;
  const right = cfg.right ?? 0;
  const headerSticky = isHeader && cfg.header;
  const shadows: string[] = [];

  if (headerSticky) shadows.push("0 4px 8px -6px var(--sticky-shadow-color)");
  if (left > 0 && index === left - 1)
    shadows.push("4px 0 8px -6px var(--sticky-shadow-color)");
  if (right > 0 && index === total - right)
    shadows.push("-4px 0 8px -6px var(--sticky-shadow-color)");

  let colStyle: CSSProperties | undefined;
  if (index < left) {
    let offset = 0;
    for (let i = 0; i < index; i++) offset += widths[i] ?? FALLBACK_WIDTH;
    colStyle = {
      position: "sticky",
      left: offset,
      zIndex: isHeader ? Z_CORNER : Z_COL,
    };
  } else if (right > 0 && index >= total - right) {
    let offset = 0;
    for (let i = index + 1; i < total; i++)
      offset += widths[i] ?? FALLBACK_WIDTH;
    colStyle = {
      position: "sticky",
      right: offset,
      zIndex: isHeader ? Z_CORNER : Z_COL,
    };
  }

  if (headerSticky) {
    return {
      position: "sticky",
      top: 0,
      ...colStyle,
      zIndex: colStyle ? Z_CORNER : Z_HEADER,
      ...(shadows.length ? { boxShadow: shadows.join(", ") } : {}),
    };
  }
  return colStyle && shadows.length
    ? { ...colStyle, boxShadow: shadows.join(", ") }
    : colStyle;
}

export function stickyCellLayoutStyle(
  index: number,
  total: number,
  widths: number[],
  cfg: StickyConfig | undefined,
  isHeader = false,
): CSSProperties {
  const width = `${widths[index] ?? FALLBACK_WIDTH}px`;
  return {
    width,
    minWidth: width,
    maxWidth: width,
    ...stickyCellStyle(index, total, widths, cfg, isHeader),
  };
}

/** True when the cell at `index` is pinned (left or right) and therefore needs an opaque bg. */
export function isStickyColumn(
  index: number,
  total: number,
  cfg: StickyConfig | undefined,
): boolean {
  if (!cfg) return false;
  const left = cfg.left ?? 0;
  const right = cfg.right ?? 0;
  return index < left || (right > 0 && index >= total - right);
}
