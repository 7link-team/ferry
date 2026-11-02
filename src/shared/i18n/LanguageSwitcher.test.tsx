import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { useTranslation } from "react-i18next";
import { afterEach, expect, it } from "vitest";
import { useRoute } from "../../app/router";
import { LanguageSwitcher } from "./LanguageSwitcher";
import { setLocale, supportedLocales } from "./locale";

afterEach(() => localStorage.clear());

it("switches accessible menu choices and route language without discarding an unsaved draft", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState(
    { draft: "retained" },
    "",
    "/zh-CN/workspaces/123/nodes/456/edit?page=2#details",
  );
  setLocale("zh-CN", false);
  let mounts = 0;
  function Draft() {
    const { t } = useTranslation();
    const route = useRoute();
    const [draft, setDraft] = useState(() => {
      mounts++;
      return "unsaved";
    });
    return (
      <>
        <LanguageSwitcher />
        <input
          aria-label="Draft"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
        <button>{t("common.save")}</button>
        <output>
          {route.workspace}/{route.id}
        </output>
      </>
    );
  }
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<Draft />));
    const input = host.querySelector("input");
    const trigger = host.querySelector<HTMLButtonElement>(
      '[aria-haspopup="menu"]',
    )!;
    expect(trigger.getAttribute("aria-label")).toBe("语言");
    expect(trigger.getAttribute("aria-description")).toBe("简体中文");
    expect(trigger.dataset.size).toBe("sm");
    await act(async () => {
      trigger.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }),
      );
    });
    const choices = [
      ...document.querySelectorAll<HTMLDivElement>('[role="menuitemradio"]'),
    ];
    expect(choices.map((choice) => choice.lang)).toEqual(supportedLocales);
    expect(
      choices
        .filter((choice) => choice.getAttribute("aria-checked") === "true")
        .map((choice) => choice.lang),
    ).toEqual(["zh-CN"]);
    await act(async () =>
      choices.find((choice) => choice.lang === "ja-JP")!.click(),
    );
    expect(trigger.getAttribute("aria-label")).toBe("言語");
    expect(trigger.getAttribute("aria-description")).toBe("日本語");
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    expect(host.querySelector("button:not([aria-haspopup])")?.textContent).toBe(
      "保存",
    );
    expect(host.querySelector("input")).toBe(input);
    expect(input?.value).toBe("unsaved");
    expect(mounts).toBe(1);
    expect(location.pathname).toBe("/ja-JP/workspaces/123/nodes/456/edit");
    expect(location.search + location.hash).toBe("?page=2#details");
    expect(history.state).toEqual({ draft: "retained" });
    expect(localStorage.getItem("ui.locale")).toBe("ja-JP");
    expect(host.querySelector("output")?.textContent).toBe("123/456");
    await act(async () => {
      history.replaceState({}, "", "/en-US/workspaces/123/nodes/456/edit");
      window.dispatchEvent(new PopStateEvent("popstate"));
    });
    expect(host.querySelector("button:not([aria-haspopup])")?.textContent).toBe(
      "Save",
    );
    expect(trigger.getAttribute("aria-description")).toBe("English");
    expect(document.documentElement.lang).toBe("en-US");
    expect(mounts).toBe(1);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
