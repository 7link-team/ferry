import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ConversationMarkdown, ConversationMessage } from "./ConversationMarkdown";

const { renderer, initialize, theme } = vi.hoisted(() => ({ renderer: vi.fn(), initialize: vi.fn(), theme: { palette: "modern", resolved: "light", customLight: {}, customDark: {} } }));
vi.mock("mermaid", () => ({ default: { initialize, render: renderer } }));
vi.mock("../../shared/theme/theme", () => ({
  useTheme: () => theme,
  createThemeColorReader: () => (_name: string, fallback: string) => fallback,
}));
vi.mock("react-i18next", async (original) => ({ ...await original<typeof import("react-i18next")>(), useTranslation: () => ({ t: (key: string) => key }) }));
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const roots: ReturnType<typeof createRoot>[] = [];
afterEach(async () => { for (const root of roots.splice(0)) await act(() => root.unmount()); document.body.replaceChildren(); vi.clearAllMocks(); theme.resolved = "light"; theme.palette = "modern"; });
async function show(text: string) {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); roots.push(root);
  await act(async () => root.render(<ConversationMarkdown>{text}</ConversationMarkdown>));
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 450)); });
  return host;
}
describe("conversation report formatting", () => {
  it("keeps report cards outside folded long answers", async () => {
    vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
    const height=vi.spyOn(HTMLElement.prototype,"scrollHeight","get").mockReturnValue(900);
    const host=document.createElement("div");document.body.append(host);const root=createRoot(host);roots.push(root);
    try {
      await act(() => root.render(<ConversationMessage endpoint="/workspaces/1/conversations/2" streaming={false}>{'Long answer\n\n[report.md](/api/workspaces/1/conversations/2/files/123)'}</ConversationMessage>));
      expect(host.querySelector('[inert]')).not.toBeNull();
      expect(host.querySelector('[data-report-card]')?.closest('[inert]')).toBeNull();
      expect(host.querySelectorAll('[data-report-card]')).toHaveLength(1);
    } finally { height.mockRestore();vi.unstubAllGlobals(); }
  });
  it("upgrades only Markdown links from this exact room to report cards", async () => {
    const host=document.createElement("div");document.body.append(host);const root=createRoot(host);roots.push(root);
    await act(() => root.render(<ConversationMarkdown endpoint="/workspaces/1/conversations/2">{'[report.md](/api/workspaces/1/conversations/2/files/123)\n\n[other.md](/api/workspaces/2/conversations/2/files/123)'}</ConversationMarkdown>));
    expect(host.querySelectorAll('[data-report-card]')).toHaveLength(1);
    expect(host.querySelector('[data-report-card] a')?.getAttribute('download')).toBe('report.md');
    expect(host.querySelector('a[href*="workspaces/2"]')?.textContent).toBe('other.md');
  });
  it("renders GFM tables and preserves ordinary code without interpreting raw HTML", async () => {
    const host = await show('| Person | Lines |\n|---|---:|\n| Alice | 12 |\n\n```json\n{"total":12}\n```\n\n<script>alert(1)</script>');
    expect(host.querySelectorAll("table tbody tr")).toHaveLength(1);
    expect(host.querySelector("td")?.textContent).toBe("Alice");
    expect(host.querySelector("pre code")?.textContent).toContain('"total":12');
    expect(host.querySelector("script")).toBeNull();
    expect(renderer).not.toHaveBeenCalled();
  });
  it("sanitizes rendered SVG and retains diagram source", async () => {
    renderer.mockResolvedValue({ svg: '<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"><text>Alice</text><script>alert(1)</script><image href="https://example.invalid/tracker"/></svg>' });
    const host = await show('```mermaid\nflowchart LR\n A[Alice] --> B[Repository]\n```');
    expect(host.querySelector("svg text")?.textContent).toBe("Alice");
    expect(host.querySelector("script, image, [onload]")).toBeNull();
    expect(host.querySelector("details code")?.textContent).toContain("A[Alice]");
  });
  it("shows source on parse errors and blocks untrusted diagram configuration", async () => {
    renderer.mockRejectedValue(new Error("invalid diagram"));
    const host = await show('```mermaid\ninvalid diagram\n```');
    expect(host.textContent).toContain("diagramError");
    expect(host.querySelector("details")?.open).toBe(true);
    const configured = await show('```mermaid\n%%{init: {"securityLevel":"loose"}}%%\nflowchart LR\nA-->B\n```');
    expect(configured.textContent).toContain("diagramError");
    expect(renderer).toHaveBeenCalledTimes(1);
  });
  it("keeps a rendered diagram visible when the conversation polls unchanged text", async () => {
    renderer.mockResolvedValue({ svg: '<svg xmlns="http://www.w3.org/2000/svg"><text>Stable diagram</text></svg>' });
    const text = '```mermaid\nflowchart LR\n A --> B\n```';
    const host = await show(text);
    expect(host.querySelector('svg text')?.textContent).toBe('Stable diagram');
    await act(async () => roots.at(-1)!.render(<ConversationMarkdown>{text}</ConversationMarkdown>));
    expect(host.querySelector('svg text')?.textContent).toBe('Stable diagram');
    expect(host.querySelector('[role="status"]')).toBeNull();
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 450)); });
    expect(renderer).toHaveBeenCalledTimes(1);
  });
  it("recolors an existing diagram after switching to a dark custom theme", async () => {
    renderer.mockResolvedValue({ svg: '<svg xmlns="http://www.w3.org/2000/svg"><text>Themed diagram</text></svg>' });
    const text = '```mermaid\nflowchart LR\n A --> B\n```';
    const host = await show(text);
    expect(initialize.mock.lastCall?.[0]).toMatchObject({ theme: "base", themeVariables: { darkMode: false } });
    theme.resolved = "dark"; theme.palette = "custom";
    await act(async () => roots.at(-1)!.render(<ConversationMarkdown>{text}</ConversationMarkdown>));
    expect(host.querySelector('svg text')?.textContent).toBe('Themed diagram');
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 100)); });
    expect(initialize.mock.lastCall?.[0]).toMatchObject({ theme: "base", themeVariables: { darkMode: true, primaryTextColor: "#fafafa", primaryColor: "#262626" } });
    expect(renderer).toHaveBeenCalledTimes(2);
    expect(host.querySelector('details code')?.textContent).toBe('flowchart LR\n A --> B');
  });
});
