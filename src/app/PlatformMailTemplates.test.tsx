import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { PlatformMailTemplates } from "./PlatformMailTemplates";

// The route store is browser-only; the real route behavior is covered by the SMTP browser test.
vi.mock("./router", () => ({ useQueryValue: (_key: string, fallback: string) => [fallback, vi.fn()] }));

it("keeps preview available and mutation controls absent in read-only mode", () => {
  const patch = vi.fn();
  const root = document.createElement("div");
  root.innerHTML = renderToStaticMarkup(<PlatformMailTemplates draft={{mail_code_subject:"Template subject",mail_code_html:"<p>{{code}}</p>"}} readOnly onPatch={patch} />);
  expect(root.querySelector("input")?.readOnly).toBe(true);
  expect(root.querySelectorAll('button[type=button]:not([role=tab])').length).toBe(0);
  const preview = root.querySelector("iframe");
  expect(preview?.getAttribute("sandbox")).toBe("");
  expect(preview?.getAttribute("srcdoc")).toContain("Template subject");
  expect(preview?.getAttribute("srcdoc")).toContain("123456");
  expect(patch).not.toHaveBeenCalled();
});
