import { expect, it } from "vitest";
import { i18nGetResources } from "@wangeditor-next/editor";
import "./RichTextValueEditor";
import messages from "../locales/richtext-domain.json";

it("covers every upstream rich text menu and preserves its interpolation parameters in all six languages", () => {
  const flatten = (value: Record<string, unknown>, prefix = ""): Record<string, string> =>
    Object.fromEntries(Object.entries(value).flatMap(([key, item]) => typeof item === "string"
      ? [[prefix + key, item]] : Object.entries(flatten(item as Record<string, unknown>, `${prefix}${key}.`))));
  const english = flatten(i18nGetResources("en"));
  expect(Object.keys(messages).sort()).toEqual(Object.keys(english).sort());
  for (const locale of ["en", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"]) {
    const translated = flatten(i18nGetResources(locale));
    expect(Object.keys(translated).sort(), locale).toEqual(Object.keys(english).sort());
    for (const [key, source] of Object.entries(english)) {
      expect(translated[key]?.trim(), `${locale}:${key}`).toBeTruthy();
      const params = (text: string) => [...text.matchAll(/\{\{(\w+)\}\}/g)].map(match => match[1]).sort();
      expect(params(translated[key]!), `${locale}:${key}`).toEqual(params(source));
    }
  }
});
