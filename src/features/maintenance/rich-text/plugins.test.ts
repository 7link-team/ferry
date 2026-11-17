import { KEYS } from "platejs";
import { createPlateEditor } from "platejs/react";
import { describe, expect, it } from "vitest";
import { sanitizeRichText } from "../value-content";
import { htmlToValue, roundTrips, valueToHtml } from "./html";
import { plugins, TABLE_ACTIONS, withIds } from "./plugins";

const TABLE = "<table><tbody><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></tbody></table>";
const action = (key: string) => TABLE_ACTIONS.flat().find(([label]) => label === key)![1];

/** An editor holding `html` with the cursor at the start of the text `at`. */
function editorAt(html: string, at: string) {
  const editor = createPlateEditor({ plugins, value: withIds(htmlToValue(html)!) });
  const [entry] = editor.api.nodes({ at: [], match: node => (node as { text?: string }).text === at });
  editor.tf.select({ path: entry![1], offset: 0 });
  return editor;
}
const html = (editor: ReturnType<typeof createPlateEditor>) => valueToHtml(editor.children);
const cells = (editor: ReturnType<typeof createPlateEditor>) =>
  [...new DOMParser().parseFromString(html(editor), "text/html").querySelectorAll("tr")].map(row => [...row.children].map(cell => cell.textContent));

describe("table editing", () => {
  it.each([
    ["richtext.rowAbove", "D", [["A", "B"], ["", ""], ["C", "D"]]],
    ["richtext.rowBelow", "B", [["A", "B"], ["", ""], ["C", "D"]]],
    ["richtext.deleteRow", "C", [["A", "B"]]],
    ["richtext.columnLeft", "B", [["A", "", "B"], ["C", "", "D"]]],
    ["richtext.columnRight", "A", [["A", "", "B"], ["C", "", "D"]]],
    ["richtext.deleteColumn", "B", [["A"], ["C"]]],
  ])("%s from the cell holding %s", (key, at, expected) => {
    const editor = editorAt(TABLE, at);
    action(key)(editor);
    expect(cells(editor)).toEqual(expected);
    // The result is still plain HTML the visual editor can reopen.
    expect(roundTrips(html(editor), sanitizeRichText)).toBe(true);
  });

  it("deletes the whole table and keeps the surrounding text", () => {
    const editor = editorAt(`<p>Before</p>${TABLE}<p>After</p>`, "A");
    action("richtext.deleteTable")(editor);
    expect(html(editor)).toBe("<p>Before</p><p>After</p>");
  });

  it("offers every row, column and table command once", () => {
    expect(TABLE_ACTIONS.flat().map(([label]) => label)).toEqual([
      "richtext.rowAbove", "richtext.rowBelow", "richtext.deleteRow",
      "richtext.columnLeft", "richtext.columnRight", "richtext.deleteColumn", "richtext.deleteTable",
    ]);
  });
});

describe("text styles", () => {
  it("saves colour, highlight and font size as inline styles", () => {
    const editor = createPlateEditor({ plugins, value: withIds(htmlToValue("<p>Hello world</p>")!) });
    editor.tf.select({ anchor: { path: [0, 0], offset: 6 }, focus: { path: [0, 0], offset: 11 } });
    editor.tf.addMarks({ [KEYS.color]: "#e03e2d", [KEYS.backgroundColor]: "#fdecc8", [KEYS.fontSize]: "18px" });
    expect(html(editor)).toBe('<p>Hello <span style="color: #e03e2d; background-color: #fdecc8; font-size: 18px;">world</span></p>');
    expect(roundTrips(html(editor), sanitizeRichText)).toBe(true);
    editor.tf.removeMarks(KEYS.backgroundColor);
    expect(html(editor)).toBe('<p>Hello <span style="color: #e03e2d; font-size: 18px;">world</span></p>');
  });

  it("inserts horizontal rules and smaller headings", () => {
    const editor = createPlateEditor({ plugins, value: withIds(htmlToValue("<p>One</p>")!) });
    editor.tf.select({ path: [0, 0], offset: 3 });
    editor.tf.toggleBlock(KEYS.h5);
    editor.tf.insertNodes([{ type: KEYS.hr, children: [{ text: "" }] }, { type: KEYS.p, children: [{ text: "Two" }] }], { at: [1] });
    expect(html(editor)).toBe("<h5>One</h5><hr><p>Two</p>");
  });
});
