/** The Plate plugins and editing commands behind the rich-text editor. */
import { BlockquotePlugin, BoldPlugin, CodePlugin, H1Plugin, H2Plugin, H3Plugin, H4Plugin, H5Plugin, H6Plugin, HorizontalRulePlugin, ItalicPlugin, StrikethroughPlugin, UnderlinePlugin } from "@platejs/basic-nodes/react";
import { FontBackgroundColorPlugin, FontColorPlugin, FontSizePlugin, TextAlignPlugin } from "@platejs/basic-styles/react";
import { CodeBlockPlugin } from "@platejs/code-block/react";
import { LinkPlugin } from "@platejs/link/react";
import { ListPlugin } from "@platejs/list-classic/react";
import { ImagePlugin } from "@platejs/media/react";
import { deleteColumn, deleteRow, deleteTable, insertTableColumn, insertTableRow } from "@platejs/table";
import { TablePlugin } from "@platejs/table/react";
import { KEYS, normalizeNodeId, type SlateEditor, type Value } from "platejs";
import { createPlatePlugin } from "platejs/react";
import type { typedText } from "../../../shared/i18n/i18n";
import { readableText } from "./contrast";
import { PLACEHOLDER } from "./html";

/**
 * How a highlight shows while editing: text the author left uncoloured gets a
 * readable colour for that highlight. Display only; the saved HTML is unchanged.
 */
export function highlightStyle({ nodeValue, text, element }: { nodeValue?: unknown; text?: object; element?: object }): Record<string, string> {
  const background = String(nodeValue);
  const node = (text ?? element ?? {}) as Record<string, unknown>;
  const color = node[KEYS.color] ? undefined : readableText(background);
  return { backgroundColor: background, ...(color ? { color } : {}) };
}

const PlaceholderPlugin = createPlatePlugin({ key: PLACEHOLDER, node: { isElement: true, isInline: true, isVoid: true } });

export const plugins = [
  H1Plugin, H2Plugin, H3Plugin, H4Plugin, H5Plugin, H6Plugin, BlockquotePlugin, HorizontalRulePlugin,
  BoldPlugin, ItalicPlugin, UnderlinePlugin, StrikethroughPlugin, CodePlugin,
  FontColorPlugin, FontSizePlugin,
  FontBackgroundColorPlugin.configure({ inject: { nodeProps: { nodeKey: KEYS.backgroundColor, transformStyle: options => highlightStyle(options) as unknown as CSSStyleDeclaration } } }),
  TextAlignPlugin.configure({ inject: { targetPlugins: [KEYS.p, KEYS.h1, KEYS.h2, KEYS.h3, KEYS.h4, KEYS.h5, KEYS.h6, KEYS.blockquote] } }),
  ListPlugin, LinkPlugin, TablePlugin, ImagePlugin, CodeBlockPlugin, PlaceholderPlugin,
];

/** Fixed colours: stored HTML must look the same in every theme and in mail clients. */
export const COLORS = [
  "#000000", "#434343", "#666666", "#999999", "#d9d9d9", "#ffffff",
  "#e03e2d", "#f39c12", "#f1c40f", "#2dc26b", "#16a085", "#3598db",
  "#1f6feb", "#8e44ad", "#c2185b", "#795548", "#fdecc8", "#d4edda",
];
export const FONT_SIZES = ["12px", "14px", "16px", "18px", "20px", "24px", "32px"];

type TableAction = [Parameters<typeof typedText>[0], (editor: SlateEditor) => void];
/** Row, column and table commands for the cell holding the selection, in menu groups. */
export const TABLE_ACTIONS: TableAction[][] = [
  [["richtext.rowAbove", editor => insertTableRow(editor, { before: true })], ["richtext.rowBelow", editor => insertTableRow(editor)], ["richtext.deleteRow", deleteRow]],
  [["richtext.columnLeft", editor => insertTableColumn(editor, { before: true })], ["richtext.columnRight", editor => insertTableColumn(editor)], ["richtext.deleteColumn", deleteColumn]],
  [["richtext.deleteTable", deleteTable]],
];

/**
 * Gives every element an id. Plate's table commands locate cells by id, and
 * values converted from HTML (opening, a new value, leaving source mode) have none.
 */
export const withIds = (value: Value): Value => normalizeNodeId(value);
