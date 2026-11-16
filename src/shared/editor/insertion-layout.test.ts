import { beforeEach, expect, it } from "vitest";
import { i18n } from "../i18n/i18n";
import { insertLayout } from "./insertion-layout";

beforeEach(async () => { await i18n.changeLanguage("zh-CN"); });

it("explains an impossible insertion in the user's language, not in internal English", () => {
  const card = (id: string, x: number) => ({ id, x, y: 0, width: 100, height: 60 });
  expect(() => insertLayout([card("a", 0)], [], "a", "missing", card("new", 0)))
    .toThrow("此节点不能插入到这条连线中：输入或输出不匹配。");
  expect(() => insertLayout([card("a", 0), card("b", 200)], [], "a", "b", card("a", 0))).toThrow(/此节点不能插入/);
  // A valid insertion still works.
  const moved = insertLayout([card("a", 0), card("b", 200)], [{ from: "a", to: "b" }], "a", "b", card("new", 0));
  expect(moved.new).toBeDefined();
});
