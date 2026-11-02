import { readdirSync, readFileSync } from "node:fs";
import { dirname, relative, resolve, sep } from "node:path";
import ts from "typescript";
import { expect, it } from "vitest";

it("keeps production dependencies acyclic and within app, feature, and shared ownership", () => {
  const root = resolve("src");
  const violations: string[] = [];
  const files = readdirSync(root, { recursive: true })
    .map(String)
    .filter((file) => /\.tsx?$/.test(file) && !file.includes(".test."));
  const absoluteFiles = new Set(files.map((file) => resolve(root, file)));
  const dependencies = new Map<string, string[]>();
  for (const file of files) {
    const absolute = resolve(root, file);
    const source = readFileSync(absolute, "utf8");
    const tree = ts.createSourceFile(
      file,
      source,
      ts.ScriptTarget.Latest,
      true,
    );
    const layer = file.split(/[\\/]/)[0];
    const targets: string[] = [];
    dependencies.set(absolute, targets);
    function check(specifier: ts.Node) {
      if (!ts.isStringLiteralLike(specifier)) return;
      const name = specifier.text;
      if (!name.startsWith(".") && !name.startsWith("@/")) return;
      const target = name.startsWith("@/")
        ? resolve(root, name.slice(2))
        : resolve(dirname(absolute), name);
      const targetLayer = relative(root, target).split(sep)[0];
      if (
        (layer === "shared" &&
          (targetLayer === "features" || targetLayer === "app")) ||
        (layer === "features" && targetLayer === "app")
      ) {
        violations.push(`${file} -> ${name}`);
      }
      const module = target.split("?")[0]!;
      const dependency = [
        module,
        `${module}.ts`,
        `${module}.tsx`,
        resolve(module, "index.ts"),
        resolve(module, "index.tsx"),
      ].find((candidate) => absoluteFiles.has(candidate));
      if (dependency) targets.push(dependency);
    }
    function visit(node: ts.Node) {
      if (
        (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
        node.moduleSpecifier
      )
        check(node.moduleSpecifier);
      if (
        ts.isCallExpression(node) &&
        node.expression.kind === ts.SyntaxKind.ImportKeyword &&
        node.arguments[0]
      )
        check(node.arguments[0]);
      if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument))
        check(node.argument.literal);
      ts.forEachChild(node, visit);
    }
    visit(tree);
  }
  expect(violations).toEqual([]);
  const complete = new Set<string>();
  const active: string[] = [];
  const cycles: string[] = [];
  function walk(file: string) {
    const start = active.indexOf(file);
    if (start !== -1) {
      cycles.push(
        [...active.slice(start), file]
          .map((path) => relative(root, path))
          .join(" -> "),
      );
      return;
    }
    if (complete.has(file)) return;
    active.push(file);
    for (const dependency of dependencies.get(file) ?? []) walk(dependency);
    active.pop();
    complete.add(file);
  }
  for (const file of absoluteFiles) walk(file);
  expect(cycles).toEqual([]);
});
