import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import ts from "typescript";
import { expect, it } from "vitest";

it("routes every page table and form through the unified components", () => {
  const root = resolve("src");
  const violations: string[] = [];
  for (const relative of readdirSync(root, { recursive: true })
    .map(String)
    .filter(
      (file) =>
        file.endsWith(".tsx") &&
        !file.includes(".test.") &&
        !/[\\/]auto-(table|form)[\\/]/.test(file),
    )) {
    const source = readFileSync(resolve(root, relative), "utf8");
    const tree = ts.createSourceFile(
      relative,
      source,
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
    function visit(node: ts.Node) {
      if (
        (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) &&
        ["table", "form"].includes(node.tagName.getText(tree))
      )
        violations.push(
          `${relative}:${tree.getLineAndCharacterOfPosition(node.getStart(tree)).line + 1}`,
        );
      ts.forEachChild(node, visit);
    }
    visit(tree);
  }
  expect(violations).toEqual([]);
});
