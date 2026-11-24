import { afterEach, expect, it } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import messages from "../../shared/i18n/locales/workflow-domain.json";
import { emptyGraph, newNode } from "./graph";

afterEach(async () => { await i18n.changeLanguage("en-US"); });

it("reports editor errors in the reader's language, never as English internals", async () => {
  await i18n.changeLanguage("zh-CN");
  expect(() => newNode(emptyGraph(), "no.such.node", "X")).toThrow("此节点类型不可用");
  for (const [key, values] of Object.entries(messages).filter(([key]) => key.startsWith("editorError."))) {
    expect(values, key).toHaveLength(6);
    // Every translation differs from the English text (no untranslated fallbacks).
    for (const value of values.slice(1, 5)) expect(value, key).not.toBe(values[0]);
  }
});

it("provides the batch current-item error in every editor locale", async () => {
  const values = messages["workflow.batchCurrentItem"];
  expect(values).toHaveLength(6);
  for (const [index, locale] of ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"].entries()) {
    await i18n.changeLanguage(locale);
    expect(i18n.t("workflow.batchCurrentItem", { ns: "workflow-domain", keySeparator: false, path: "/nodes/1/params/prompt" }))
      .toBe(values[index]!.replace("{{path}}", "/nodes/1/params/prompt"));
  }
});
