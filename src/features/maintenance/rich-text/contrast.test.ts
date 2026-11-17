import { KEYS } from "platejs";
import { createPlateEditor } from "platejs/react";
import { expect, it } from "vitest";
import { luminance, parseColor, readableText } from "./contrast";
import { canonicalHtml, htmlToValue, valueToHtml } from "./html";
import { highlightStyle, plugins, withIds } from "./plugins";

it("reads hex and rgb colours", () => {
  expect(parseColor("#ff0")).toEqual([255, 255, 0, 1]);
  expect(parseColor("#1F6FEB")).toEqual([31, 111, 235, 1]);
  expect(parseColor("rgb(255, 236, 61)")).toEqual([255, 236, 61, 1]);
  expect(parseColor("rgba(0, 0, 0, 0.2)")).toEqual([0, 0, 0, 0.2]);
  expect(parseColor("rgb(0 0 0 / 50%)")).toEqual([0, 0, 0, 0.5]);
  expect(parseColor("yellow")).toBeUndefined();
  expect(luminance([255, 255, 255])).toBeCloseTo(1);
  expect(luminance([0, 0, 0])).toBe(0);
});

it.each([
  ["rgb(255, 236, 61)", "#111111"], // wangEditor yellow highlight
  ["#fdecc8", "#111111"],
  ["#ffffff", "#111111"],
  ["#2dc26b", "#111111"],
  ["#000000", "#ffffff"],
  ["#1f6feb", "#ffffff"],
  ["#8e44ad", "#ffffff"],
  ["rgb(225, 60, 57)", "#111111"], // mid red: black edges out white
])("text on %s is %s", (background, text) => {
  expect(readableText(background)).toBe(text);
});

it("leaves unknown and see-through highlights to the theme", () => {
  expect(readableText("yellow")).toBeUndefined();
  expect(readableText("rgba(255, 255, 0, 0.1)")).toBeUndefined();
});

it("adds a readable text colour only where the author chose none, and never saves it", () => {
  expect(highlightStyle({ nodeValue: "rgb(255, 236, 61)", text: { text: "x", backgroundColor: "rgb(255, 236, 61)" } }))
    .toEqual({ backgroundColor: "rgb(255, 236, 61)", color: "#111111" });
  expect(highlightStyle({ nodeValue: "#000000", text: { text: "x", backgroundColor: "#000000", color: "#e03e2d" } }))
    .toEqual({ backgroundColor: "#000000" });
  const html = '<p><span style="background-color: rgb(255, 236, 61);">{{ run.branch }} ok</span></p>';
  const editor = createPlateEditor({ plugins, value: withIds(htmlToValue(html)!) });
  expect(canonicalHtml(valueToHtml(editor.children))).toBe(canonicalHtml(html));
  expect(JSON.stringify(editor.children)).not.toContain(`"${KEYS.color}"`);
});
