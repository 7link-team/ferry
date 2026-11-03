import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { TagBadge } from "./TagBadge";

it("uses the saved background with readable text and updates the same badge", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  const root = createRoot(host);
  try {
    await act(async () => root.render(<TagBadge color="#111111">Production</TagBadge>));
    expect(host.firstElementChild?.getAttribute("style")).toContain("background-color: rgb(17, 17, 17)");
    expect(host.firstElementChild?.getAttribute("style")).toContain("color: rgb(255, 255, 255)");
    expect(host.firstElementChild?.classList.contains("rounded-full")).toBe(true);
    await act(async () => root.render(<TagBadge color="#fff">Production</TagBadge>));
    expect(host.firstElementChild?.getAttribute("style")).toContain("color: rgb(0, 0, 0)");
  } finally { await act(async () => root.unmount()); }
});
