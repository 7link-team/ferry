export interface LayoutCard { id: string; x: number; y: number; width: number; height: number }

/** Reserve an inline slot by shifting the target row/column and successors. */
export function insertLayout(cards: LayoutCard[], links: { from: string; to: string }[], fromId: string, toId: string, inserted: LayoutCard, direction: "right" | "down" = "right"): Record<string, { x: number; y: number }> {
  const from = cards.find(card => card.id === fromId), to = cards.find(card => card.id === toId);
  if (!from || !to || cards.some(card => card.id === inserted.id)) throw new Error("Invalid insertion layout");
  const axis = direction === "right" ? "x" : "y", dimension = direction === "right" ? "width" : "height";
  const position = { x: from.x, y: from.y };
  position[axis] = Math.max(from[axis] + from[dimension] + 96, (from[axis] + from[dimension] + to[axis] - inserted[dimension]) / 2);
  const overlaps = cards.filter(card => card.id !== from.id && position.x < card.x + card.width + 48 && position.x + inserted.width + 48 > card.x && position.y < card.y + card.height + 48 && position.y + inserted.height + 48 > card.y);
  const boundary = Math.min(to[axis], ...overlaps.map(card => card[axis]));
  const shift = Math.max(0, position[axis] + inserted[dimension] + 96 - boundary);
  const successors = new Set([to.id]);
  for (let changed = true; changed;) {
    changed = false;
    for (const link of links) if (successors.has(link.from) && !successors.has(link.to)) { successors.add(link.to); changed = true; }
  }
  return Object.fromEntries([...cards.map(card => [card.id, { x: card.x + (axis === "x" && card.id !== from.id && (card.x >= boundary || successors.has(card.id)) ? shift : 0), y: card.y + (axis === "y" && card.id !== from.id && (card.y >= boundary || successors.has(card.id)) ? shift : 0) }] as const), [inserted.id, position]]);
}
