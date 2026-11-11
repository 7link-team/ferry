import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { AvatarEditor, IdentityAvatar, avatarHash } from "./identity-avatar";

it("generates stable distinct human and robot avatars without network requests", () => {
  const human = renderToStaticMarkup(<IdentityAvatar id="1" name="User" kind="user" />);
  expect(renderToStaticMarkup(<IdentityAvatar id="1" name="User" kind="user" />)).toBe(human);
  expect(renderToStaticMarkup(<IdentityAvatar id="1" name="Agent" kind="agent" />)).not.toBe(human);
  expect(renderToStaticMarkup(<IdentityAvatar id="2" name="User" kind="user" />)).not.toBe(human);
  expect(human).not.toContain("<img");
  expect(avatarHash("user:1")).toBe(avatarHash("user:1"));
});
it("renders emoji as text and never executable markup", () => {
  expect(renderToStaticMarkup(<IdentityAvatar id="1" name="User" kind="user" value={{ style: "emoji", emoji: "<script>" }} />)).toContain("&lt;script&gt;");
});
it("rejects an unsupported upload without changing the saved avatar", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); const change = vi.fn();
  try {
    await act(async () => root.render(<AvatarEditor id="1" name="User" kind="user" onChange={change} />));
    const input = host.querySelector("input")!;
    Object.defineProperty(input, "files", { value: [new File(["<svg/>"], "a.svg", { type: "image/svg+xml" })] });
    await act(async () => input.dispatchEvent(new Event("change", { bubbles: true })));
    expect(host.querySelector('[role="alert"]')).not.toBeNull();
    expect(change).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); }
});
