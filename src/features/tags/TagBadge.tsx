import type { ReactNode } from "react";
import { Badge } from "../../shared/ui/badge";

/** Keep saved tag colors consistent in lists, details and selection controls. */
export function TagBadge({ color, children }: { color?: string | undefined; children: ReactNode }) {
  const hex = color?.match(/^#([\da-f]{3}|[\da-f]{6})$/i)?.[1];
  const expanded = hex?.length === 3 ? [...hex].map(value => value + value).join("") : hex;
  const rgb = expanded ? [0, 2, 4].map(offset => parseInt(expanded.slice(offset, offset + 2), 16) / 255) : [];
  const luminance = rgb.reduce((sum, channel, index) => sum + (channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4) * [0.2126, 0.7152, 0.0722][index]!, 0);
  return <Badge variant="secondary" className="rounded-full" style={color ? { backgroundColor: color, color: expanded ? (luminance > 0.179 ? "#000000" : "#ffffff") : undefined } : undefined}>{children}</Badge>;
}
