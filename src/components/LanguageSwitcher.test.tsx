import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it } from "vitest";
import { useTranslation } from "react-i18next";
import { LanguageSwitcher } from "./LanguageSwitcher";
import { setLocale } from "../locale";
import { useRoute } from "../router";

afterEach(() => localStorage.clear());

it("switches accessible controls and route language without discarding an unsaved draft", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState({}, "", "/zh-CN/workspaces/123/nodes/456/edit?page=2#details");
  setLocale("zh-CN", false);
  let mounts = 0;
  function Draft() {
    const { t } = useTranslation();
    const route = useRoute();
    const [draft, setDraft] = useState(() => { mounts++; return "unsaved"; });
    return <><LanguageSwitcher /><input aria-label="Draft" value={draft} onChange={event => setDraft(event.target.value)} /><button>{t("common.save")}</button><output>{route.workspace}/{route.id}</output></>;
  }
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<Draft />));
    const input = host.querySelector("input");
    const select = host.querySelector("select")!;
    expect(select.getAttribute("aria-label")).toBe("语言");
    expect(select.options).toHaveLength(6);
    await act(async () => { select.value = "ja-JP"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(select.getAttribute("aria-label")).toBe("言語");
    expect(host.querySelector("button")?.textContent).toBe("保存");
    expect(host.querySelector("input")).toBe(input);
    expect(input?.value).toBe("unsaved");
    expect(mounts).toBe(1);
    expect(location.pathname).toBe("/ja-JP/workspaces/123/nodes/456/edit");
    expect(location.search + location.hash).toBe("?page=2#details");
    expect(host.querySelector("output")?.textContent).toBe("123/456");
    await act(async () => { history.replaceState({}, "", "/en-US/workspaces/123/nodes/456/edit"); window.dispatchEvent(new PopStateEvent("popstate")); });
    expect(host.querySelector("button")?.textContent).toBe("Save");
    expect(document.documentElement.lang).toBe("en-US");
    expect(mounts).toBe(1);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
