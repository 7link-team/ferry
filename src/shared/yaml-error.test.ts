import { parseDocument } from "yaml";
import { beforeEach, expect, it } from "vitest";
import { i18n } from "./i18n/i18n";
import { YamlSyntaxError, yamlError } from "./yaml-error";
import { parseActions } from "../features/pipelines/actions-workflow";
import { readManifest } from "../features/deployments/manifest/model";

beforeEach(async () => { await i18n.changeLanguage("zh-CN"); });

const problem = (source: string) => yamlError(parseDocument(source, { uniqueKeys: true }).errors);

it("names the line, column and a plain reason, and keeps the parser text as detail", () => {
  const duplicate = problem("name: a\nname: b\n");
  expect(duplicate).toBeInstanceOf(YamlSyntaxError);
  expect(duplicate.line).toBe(2);
  expect(duplicate.column).toBe(1);
  expect(duplicate.message).toBe("YAML 第 2 行第 1 列：同一层级出现了重复的键");
  expect(duplicate.detail).toMatch(/unique/i);
  expect(duplicate.message).not.toMatch(/[A-Za-z]{6,}/);

  expect(problem("jobs:\n\tbuild: x\n").message).toContain("不能用制表符缩进");
  expect(problem("a: \"open\n").message).toContain("引号或括号没有闭合");
  expect(problem("a:\n  b: 1\n c: 2\n").message).toMatch(/^YAML 第 3 行第 \d+ 列：/);
});

it("parsers surface the localized message instead of the library's English text", async () => {
  expect(() => parseActions("jobs:\n  build: {\n")).toThrow(YamlSyntaxError);
  expect(() => parseActions("jobs:\n  a: 1\n  a: 2\n")).toThrow("YAML 第 3 行第 3 列：同一层级出现了重复的键");
  expect(() => readManifest("metadata: {}\nmetadata: {}\n")).toThrow(/^YAML 第 2 行第 1 列：/);
  await i18n.changeLanguage("en-US");
  expect(problem("a: 1\na: 2\n").message).toBe("YAML line 2, column 1: a key appears twice in the same mapping");
});
