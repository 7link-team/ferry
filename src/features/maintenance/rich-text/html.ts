/**
 * Stored rich text is plain HTML (email bodies, footers, settings), so the
 * editor converts it to and from Plate nodes itself: Plate's own serializer
 * writes editor-specific wrappers that do not belong in stored HTML.
 *
 * Only a known subset is converted. Anything else (inline styles other than
 * text alignment, unknown tags, image sizes, ...) makes `htmlToValue` return
 * null, and `roundTrips` checks that a conversion keeps everything the
 * sanitizer keeps, so the editor never silently drops formatting.
 */
import { KEYS, type Descendant, type TElement, type TText, type Value } from "platejs";

/** Inline, non-editable template variable such as `{{ user.name }}`. */
export const PLACEHOLDER = "placeholder";
export const PLACEHOLDER_PATTERN = /\{\{\s*[\w.]+\s*\}\}/g;
export const isPlaceholder = (text: string) => new RegExp(`^${PLACEHOLDER_PATTERN.source}$`).test(text);

export const MARKS = [KEYS.bold, KEYS.italic, KEYS.underline, KEYS.strikethrough, KEYS.code] as const;
export type Mark = (typeof MARKS)[number];
const MARK_TAG: Record<Mark, string> = { bold: "strong", italic: "em", underline: "u", strikethrough: "s", code: "code" };
const TAG_MARK: Record<string, Mark> = { strong: "bold", b: "bold", em: "italic", i: "italic", u: "underline", s: "strikethrough", strike: "strikethrough", del: "strikethrough", code: "code" };
export const ALIGNMENTS = ["left", "center", "right", "justify"] as const;
const TEXT_BLOCKS = new Set<string>([KEYS.p, KEYS.h1, KEYS.h2, KEYS.h3, KEYS.h4, KEYS.h5, KEYS.h6, KEYS.blockquote]);
/** Inline styles kept as text marks (the CSS property and the Plate mark). */
export const STYLE_MARKS = { color: KEYS.color, "background-color": KEYS.backgroundColor, "font-size": KEYS.fontSize } as const;
const STYLE_MARK_CSS = Object.fromEntries(Object.entries(STYLE_MARKS).map(([css, mark]) => [mark, css])) as Record<string, string>;
/** Image size styles kept as image properties. */
const IMAGE_STYLES = ["width", "height"] as const;

class Unsupported extends Error {}
const unsupported = (): never => { throw new Unsupported(); };

function body(html: string): HTMLElement {
  return new DOMParser().parseFromString(`<!doctype html><body>${html}</body>`, "text/html").body;
}

/** Declarations of a style attribute, lower-cased, without empty ones. */
function styles(element: Element): Map<string, string> {
  const result = new Map<string, string>();
  for (const declaration of (element.getAttribute("style") ?? "").split(";")) {
    const at = declaration.indexOf(":");
    if (at < 0) { if (declaration.trim()) result.set(declaration.trim().toLowerCase(), ""); continue; }
    const name = declaration.slice(0, at).trim().toLowerCase();
    const value = declaration.slice(at + 1).trim().toLowerCase();
    if (name) result.set(name, value);
  }
  return result;
}

/** Attributes that carry no information and are dropped on both sides. */
function ignorable(element: Element, name: string, value: string): boolean {
  const v = value.trim().toLowerCase();
  if (!v && name !== "alt") return true;
  if ((name === "colspan" || name === "rowspan") && v === "1") return true;
  if (name === "width" && v === "auto") return true;
  if (name === "style") {
    const own = [...styles(element)].filter(([key, val]) => !(key === "width" && val === "auto"));
    return own.length === 0;
  }
  return false;
}

/** The element's attributes that matter; throws for any outside `allowed`. */
function attributes(element: Element, allowed: readonly string[]): Map<string, string> {
  const result = new Map<string, string>();
  for (const { name, value } of [...element.attributes]) {
    if (ignorable(element, name, value)) continue;
    if (!allowed.includes(name)) unsupported();
    result.set(name, value);
  }
  return result;
}

/** text-align from the style attribute; any other style is unsupported. */
function alignment(element: Element, attrs: Map<string, string>): string | undefined {
  if (!attrs.has("style")) return undefined;
  const own = [...styles(element)].filter(([key, val]) => !(key === "width" && val === "auto"));
  if (own.length !== 1 || own[0]![0] !== "text-align" || !(ALIGNMENTS as readonly string[]).includes(own[0]![1])) unsupported();
  return own[0]![1] === "left" ? undefined : own[0]![1];
}

const isBlockTag = (tag: string) => ["p", "h1", "h2", "h3", "blockquote", "pre", "ul", "ol", "table", "div", "h4", "h5", "h6", "hr"].includes(tag);
const isWhitespace = (node: Node) => node.nodeType === Node.TEXT_NODE && !/\S/.test(node.textContent ?? "");

type Marks = Partial<Record<Mark, true>> & Record<string, true | string>;

/** Text leaves (and placeholder chips) for one run of text with marks. */
function textNodes(text: string, marks: Marks): Descendant[] {
  const result: Descendant[] = [];
  let at = 0;
  for (const match of text.matchAll(PLACEHOLDER_PATTERN)) {
    if (match.index! > at) result.push({ text: text.slice(at, match.index), ...marks });
    result.push({ type: PLACEHOLDER, name: match[0], ...marks, children: [{ text: "" }] } as TElement);
    at = match.index! + match[0].length;
  }
  if (at < text.length || result.length === 0) result.push({ text: text.slice(at), ...marks });
  return result;
}

function inline(nodes: Iterable<Node>, marks: Marks = {}): Descendant[] {
  const result: Descendant[] = [];
  for (const node of nodes) {
    if (node.nodeType === Node.TEXT_NODE) { result.push(...textNodes((node.textContent ?? "").replace(/\s+/g, " "), marks)); continue; }
    if (node.nodeType !== Node.ELEMENT_NODE) continue;
    const element = node as Element;
    const tag = element.tagName.toLowerCase();
    if (tag === "br") { result.push({ text: "\n", ...marks }); continue; }
    if (TAG_MARK[tag]) { attributes(element, []); result.push(...inline(element.childNodes, { ...marks, [TAG_MARK[tag]!]: true })); continue; }
    if (tag === "span") {
      const attrs = attributes(element, ["style"]);
      const styled: Record<string, string> = {};
      if (attrs.has("style")) {
        for (const [property, value] of styles(element)) {
          const mark = STYLE_MARKS[property as keyof typeof STYLE_MARKS];
          if (!mark || !value) unsupported();
          styled[mark] = value;
        }
      }
      result.push(...inline(element.childNodes, { ...marks, ...styled }));
      continue;
    }
    if (tag === "a") {
      const attrs = attributes(element, ["href", "target", "rel"]);
      result.push({ type: KEYS.link, url: attrs.get("href") ?? "", ...(attrs.has("target") ? { target: attrs.get("target") } : {}), ...(attrs.has("rel") ? { rel: attrs.get("rel") } : {}), children: nonEmpty(inline(element.childNodes, marks)) } as TElement);
      continue;
    }
    unsupported();
  }
  return result;
}

/** Collapse spaces at a block's edges and keep at least one leaf. */
function trimEdges(children: Descendant[]): Descendant[] {
  const first = children[0], last = children.at(-1);
  if (first && "text" in first) (first as TText).text = (first as TText).text.replace(/^ /, "");
  if (last && "text" in last) (last as TText).text = (last as TText).text.replace(/ $/, "");
  return nonEmpty(children);
}
const nonEmpty = (children: Descendant[]) => children.length ? children : [{ text: "" }];

function image(element: Element): TElement {
  const attrs = attributes(element, ["src", "alt", "width", "height", "style"]);
  if (!attrs.get("src")) unsupported();
  const css: Record<string, string> = {};
  if (attrs.has("style")) {
    for (const [property, value] of styles(element)) {
      if (property === "width" && value === "auto") continue;
      if (!(IMAGE_STYLES as readonly string[]).includes(property) || !value) unsupported();
      css[property] = value;
    }
  }
  return {
    type: KEYS.img, url: attrs.get("src")!,
    ...(attrs.has("alt") ? { alt: attrs.get("alt") } : {}),
    ...(attrs.has("width") ? { attrWidth: attrs.get("width") } : {}),
    ...(attrs.has("height") ? { attrHeight: attrs.get("height") } : {}),
    ...(css.width ? { cssWidth: css.width } : {}),
    ...(css.height ? { cssHeight: css.height } : {}),
    children: [{ text: "" }],
  };
}

/** Block nodes for a container's children; loose inline content becomes a paragraph. */
function blocks(nodes: Iterable<Node>): TElement[] {
  const result: TElement[] = [];
  let loose: Node[] = [];
  const flush = () => {
    if (loose.some(node => !isWhitespace(node))) result.push({ type: KEYS.p, children: trimEdges(inline(loose)) });
    loose = [];
  };
  for (const node of nodes) {
    const element = node.nodeType === Node.ELEMENT_NODE ? node as Element : null;
    const tag = element?.tagName.toLowerCase() ?? "";
    if (!element || !(isBlockTag(tag) || tag === "img")) { loose.push(node); continue; }
    flush();
    result.push(block(element, tag));
  }
  flush();
  return result;
}

function block(element: Element, tag: string): TElement {
  if (tag === "img") return image(element);
  if (tag === "hr") { attributes(element, []); return { type: KEYS.hr, children: [{ text: "" }] }; }
  if (["p", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote"].includes(tag)) {
    const attrs = attributes(element, ["style"]);
    const align = alignment(element, attrs);
    const children = [...element.childNodes].filter(node => !isWhitespace(node));
    // A paragraph holding only an image is the editor's block image.
    if (tag === "p" && !align && children.length === 1 && children[0]!.nodeName === "IMG") return image(children[0] as Element);
    if ([...element.children].some(child => isBlockTag(child.tagName.toLowerCase()) || child.tagName === "IMG")) unsupported();
    return { type: tag, ...(align ? { align } : {}), children: trimEdges(inline(element.childNodes)) };
  }
  if (tag === "pre") {
    attributes(element, []);
    const children = [...element.childNodes];
    const code = children.length === 1 && children[0]!.nodeName === "CODE" ? children[0] as Element : null;
    if (!code && children.some(node => node.nodeType !== Node.TEXT_NODE)) unsupported();
    const attrs = code ? attributes(code, ["class"]) : new Map<string, string>();
    const language = attrs.get("class")?.match(/^language-([\w+-]+)$/)?.[1];
    if (attrs.has("class") && !language) unsupported();
    if (code && [...code.childNodes].some(node => node.nodeType !== Node.TEXT_NODE)) unsupported();
    const text = (code ?? element).textContent ?? "";
    return { type: KEYS.codeBlock, ...(language ? { lang: language } : {}), children: text.replace(/\n$/, "").split("\n").map(line => ({ type: KEYS.codeLine, children: [{ text: line }] })) };
  }
  if (tag === "ul" || tag === "ol") {
    attributes(element, []);
    const items: TElement[] = [];
    for (const child of element.childNodes) {
      if (isWhitespace(child)) continue;
      if (child.nodeName !== "LI") unsupported();
      items.push(listItem(child as Element));
    }
    return { type: tag === "ul" ? KEYS.ulClassic : KEYS.olClassic, children: items };
  }
  if (tag === "table") {
    attributes(element, []);
    const rows: TElement[] = [];
    const collect = (parent: Element) => {
      for (const child of parent.children) {
        const name = child.tagName.toLowerCase();
        if (["thead", "tbody", "tfoot"].includes(name)) { attributes(child, []); collect(child); continue; }
        if (name !== "tr") unsupported();
        attributes(child, []);
        const cells: TElement[] = [];
        for (const cell of child.children) {
          const cellTag = cell.tagName.toLowerCase();
          if (cellTag !== "td" && cellTag !== "th") unsupported();
          const attrs = attributes(cell, ["colspan", "rowspan"]);
          const span = (key: string) => attrs.has(key) ? Number(attrs.get(key)) : undefined;
          const colSpan = span("colspan"), rowSpan = span("rowspan");
          if ([colSpan, rowSpan].some(value => value !== undefined && !(Number.isInteger(value) && value > 0))) unsupported();
          const content = blocks(cell.childNodes);
          cells.push({ type: cellTag === "td" ? KEYS.td : KEYS.th, ...(colSpan ? { colSpan } : {}), ...(rowSpan ? { rowSpan } : {}), children: content.length ? content : [{ type: KEYS.p, children: [{ text: "" }] }] });
        }
        if (!cells.length) unsupported();
        rows.push({ type: KEYS.tr, children: cells });
      }
    };
    collect(element);
    if (!rows.length) unsupported();
    return { type: KEYS.table, children: rows };
  }
  return unsupported();
}

function listItem(element: Element): TElement {
  const attrs = attributes(element, []);
  void attrs;
  const content: Node[] = [];
  const nested: TElement[] = [];
  const children = [...element.childNodes].filter(node => !isWhitespace(node));
  // `<li><p>text</p></li>` is the same item as `<li>text</li>`.
  const unwrapped = children.length >= 1 && children[0]!.nodeName === "P" && (children[0] as Element).attributes.length === 0
    ? [...children[0]!.childNodes, ...children.slice(1)] : children;
  for (const node of unwrapped) {
    const tag = node.nodeName.toLowerCase();
    if (tag === "ul" || tag === "ol") { nested.push(block(node as Element, tag)); continue; }
    if (nested.length || isBlockTag(tag)) unsupported();
    content.push(node);
  }
  return { type: KEYS.li, children: [{ type: KEYS.lic, children: trimEdges(inline(content)) }, ...nested] };
}

/** Plate nodes for stored HTML, or null when it uses anything outside the supported subset. */
export function htmlToValue(html: string): Value | null {
  try {
    const result = blocks(body(html).childNodes);
    return result.length ? result : [{ type: KEYS.p, children: [{ text: "" }] }];
  } catch (error) {
    if (error instanceof Unsupported) return null;
    throw error;
  }
}

// ---------------------------------------------------------------- serialize

const escapeText = (text: string) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const escapeAttribute = (text: string) => escapeText(text).replace(/"/g, "&quot;");

function wrapMarks(html: string, node: Record<string, unknown>): string {
  const marked = [...MARKS].reverse().reduce((inner, mark) => node[mark] ? `<${MARK_TAG[mark]}>${inner}</${MARK_TAG[mark]}>` : inner, html);
  const style = Object.values(STYLE_MARKS).filter(mark => typeof node[mark] === "string" && node[mark])
    .map(mark => `${STYLE_MARK_CSS[mark]}: ${node[mark]};`).join(" ");
  return style ? `<span style="${escapeAttribute(style)}">${marked}</span>` : marked;
}

function serializeInline(children: Descendant[]): string {
  return children.map(child => {
    if ("text" in child) { const leaf = child as TText; return leaf.text ? wrapMarks(escapeText(leaf.text).replace(/\n/g, "<br>"), leaf) : ""; }
    const element = child as TElement;
    if (element.type === PLACEHOLDER) return wrapMarks(escapeText(String(element.name ?? "")), element);
    if (element.type === KEYS.link) {
      const attrs = [`href="${escapeAttribute(String(element.url ?? ""))}"`];
      if (element.target) attrs.push(`target="${escapeAttribute(String(element.target))}"`);
      if (element.rel) attrs.push(`rel="${escapeAttribute(String(element.rel))}"`);
      return `<a ${attrs.join(" ")}>${serializeInline(element.children)}</a>`;
    }
    return serializeInline(element.children);
  }).join("");
}

const alignAttribute = (element: TElement) => element.align && element.align !== "left" ? ` style="text-align: ${escapeAttribute(String(element.align))};"` : "";

function serializeBlock(element: TElement): string {
  const type = element.type;
  if (TEXT_BLOCKS.has(type)) {
    const inner = serializeInline(element.children);
    return `<${type}${alignAttribute(element)}>${inner || "<br>"}</${type}>`;
  }
  if (type === KEYS.hr) return "<hr>";
  if (type === KEYS.img) {
    const attrs = [`src="${escapeAttribute(String(element.url ?? ""))}"`];
    if (element.alt !== undefined) attrs.push(`alt="${escapeAttribute(String(element.alt))}"`);
    if (element.attrWidth) attrs.push(`width="${escapeAttribute(String(element.attrWidth))}"`);
    if (element.attrHeight) attrs.push(`height="${escapeAttribute(String(element.attrHeight))}"`);
    const css = [element.cssWidth ? `width: ${element.cssWidth};` : "", element.cssHeight ? `height: ${element.cssHeight};` : ""].filter(Boolean).join(" ");
    if (css) attrs.push(`style="${escapeAttribute(css)}"`);
    return `<img ${attrs.join(" ")}>`;
  }
  if (type === KEYS.codeBlock) {
    const lines = element.children.map(line => (line as TElement).children.map(leaf => "text" in leaf ? leaf.text : "").join(""));
    return `<pre><code${element.lang ? ` class="language-${escapeAttribute(String(element.lang))}"` : ""}>${escapeText(lines.join("\n"))}</code></pre>`;
  }
  if (type === KEYS.ulClassic || type === KEYS.olClassic) return `<${type}>${element.children.map(item => serializeBlock(item as TElement)).join("")}</${type}>`;
  if (type === KEYS.li) {
    return `<li>${(element.children as TElement[]).map(child => child.type === KEYS.lic ? serializeInline(child.children) : serializeBlock(child)).join("")}</li>`;
  }
  if (type === KEYS.lic) return serializeInline(element.children);
  if (type === KEYS.table) return `<table><tbody>${element.children.map(row => serializeBlock(row as TElement)).join("")}</tbody></table>`;
  if (type === KEYS.tr) return `<tr>${element.children.map(cell => serializeBlock(cell as TElement)).join("")}</tr>`;
  if (type === KEYS.td || type === KEYS.th) {
    const spans = `${Number(element.colSpan) > 1 ? ` colspan="${element.colSpan}"` : ""}${Number(element.rowSpan) > 1 ? ` rowspan="${element.rowSpan}"` : ""}`;
    const content = element.children as TElement[];
    // A single plain paragraph is written as the cell's own text.
    const inner = content.length === 1 && content[0]!.type === KEYS.p && !content[0]!.align
      ? serializeInline(content[0]!.children) : content.map(serializeBlock).join("");
    return `<${type}${spans}>${inner}</${type}>`;
  }
  return serializeInline(element.children);
}

/** Stored HTML for Plate nodes. A document holding one empty paragraph is "". */
export function valueToHtml(value: Value): string {
  const nodes = value as TElement[];
  if (nodes.length === 1 && nodes[0]!.type === KEYS.p && !nodes[0]!.align && !serializeInline(nodes[0]!.children)) return "";
  return nodes.map(serializeBlock).join("");
}

// ---------------------------------------------------------------- compare

/** A canonical description of HTML: equal strings mean the same document. */
export function canonicalHtml(html: string): string {
  const blocksOf = (nodes: Iterable<Node>): string[] => {
    const out: string[] = [];
    let loose: Node[] = [];
    const flush = () => { if (loose.some(node => !isWhitespace(node))) out.push(`p[]{${inlineOf(loose, true)}}`); loose = []; };
    for (const node of nodes) {
      const element = node.nodeType === Node.ELEMENT_NODE ? node as Element : null;
      const tag = element?.tagName.toLowerCase() ?? "";
      if (!element || !(isBlockTag(tag) || ["img", "li", "tr", "td", "th", "thead", "tbody", "tfoot"].includes(tag))) { loose.push(node); continue; }
      flush();
      out.push(blockOf(element, tag));
    }
    flush();
    return out;
  };
  const attrsOf = (element: Element) => [...element.attributes]
    .filter(({ name, value }) => !ignorable(element, name, value))
    .map(({ name, value }) => name === "style"
      ? `style=${[...styles(element)].filter(([key, val]) => !(key === "width" && val === "auto")).map(([key, val]) => `${key}:${val}`).sort().join(";")}`
      : `${name}=${value}`)
    .filter(entry => entry !== "style=text-align:left")
    .sort().join(",");
  const blockOf = (element: Element, tag: string): string => {
    if (["thead", "tbody", "tfoot"].includes(tag) && !attrsOf(element)) return blocksOf(element.children).join("");
    const children = [...element.childNodes].filter(node => !isWhitespace(node));
    if (tag === "p" && !attrsOf(element) && children.length === 1 && children[0]!.nodeName === "IMG") return blockOf(children[0] as Element, "img");
    if (tag === "pre") return `pre[${attrsOf(element)}]{${[...element.childNodes].map(node => node.nodeType === Node.ELEMENT_NODE ? `${node.nodeName.toLowerCase()}[${attrsOf(node as Element)}]${JSON.stringify((node.textContent ?? "").replace(/\n$/, ""))}` : JSON.stringify((node.textContent ?? "").replace(/\n$/, ""))).join("")}}`;
    const inner = ["ul", "ol", "table", "tr", "td", "th", "li"].includes(tag) ? blocksOf(element.childNodes).join("") : inlineOf(element.childNodes, true);
    return `${tag}[${attrsOf(element)}]{${inner}}`;
  };
  const inlineOf = (nodes: Iterable<Node>, edges: boolean): string => {
    const tokens: string[] = [];
    let text = "", marks = "";
    const push = () => { if (text) tokens.push(`${marks}:${JSON.stringify(text)}`); text = ""; };
    const walk = (list: Iterable<Node>, active: Set<string>) => {
      for (const node of list) {
        const key = [...active].sort().join("+");
        if (node.nodeType === Node.TEXT_NODE) {
          if (key !== marks) { push(); marks = key; }
          text += (node.textContent ?? "").replace(/\s+/g, " ");
          continue;
        }
        if (node.nodeType !== Node.ELEMENT_NODE) continue;
        const element = node as Element, tag = element.tagName.toLowerCase();
        if (tag === "br") { if (key !== marks) { push(); marks = key; } text += "\n"; continue; }
        if (TAG_MARK[tag] && !attrsOf(element)) { walk(element.childNodes, new Set([...active, TAG_MARK[tag]!])); continue; }
        if (tag === "span" && !attrsOf(element)) { walk(element.childNodes, active); continue; }
        if (tag === "span" && [...element.attributes].every(({ name, value }) => name === "style" || ignorable(element, name, value))) {
          const declared = [...styles(element)];
          if (declared.length && declared.every(([property, value]) => property in STYLE_MARKS && value)) {
            // An inner span overrides the same property of an outer one.
            const kept = [...active].filter(mark => !declared.some(([property]) => mark.startsWith(`${property}=`)));
            walk(element.childNodes, new Set([...kept, ...declared.map(([property, value]) => `${property}=${value}`)]));
            continue;
          }
        }
        push();
        tokens.push(`${tag}[${attrsOf(element)}](${inlineOf(element.childNodes, false)})`);
      }
    };
    walk(nodes, new Set());
    push();
    if (edges && tokens.length) {
      // Spaces at a block's edges and a lone trailing <br> are not content.
      tokens[0] = tokens[0]!.replace(/^([^:]*):"( )/, '$1:"');
      tokens[tokens.length - 1] = tokens.at(-1)!.replace(/ "$/, '"').replace(/\\n"$/, '"');
    }
    return tokens.filter(token => !/:""$/.test(token)).join("");
  };
  const result = blocksOf(body(html).childNodes);
  return result.length === 1 && result[0] === "p[]{}" ? "" : result.join("");
}

/**
 * Whether the visual editor can hold this HTML without changing it: the
 * supported subset converts, and converting back keeps everything the
 * sanitizer keeps.
 */
export function roundTrips(html: string, sanitize: (html: string) => string): boolean {
  const clean = sanitize(html);
  const value = htmlToValue(clean);
  if (!value) return false;
  return canonicalHtml(clean) === canonicalHtml(sanitize(valueToHtml(value)));
}
