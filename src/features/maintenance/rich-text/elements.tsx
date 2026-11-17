/** How each rich-text node looks while editing. Theme tokens only, so every palette and dark mode apply. */
import { KEYS, type TElement } from "platejs";
import { PlateElement, PlateLeaf, type PlateElementProps, type PlateLeafProps } from "platejs/react";
import { typedText } from "../../../shared/i18n/i18n";
import { PLACEHOLDER } from "./html";

// The tag only changes what is rendered; the props are the same for every tag.
const element = (as: keyof HTMLElementTagNameMap, className: string) =>
  function Element(props: PlateElementProps) {
    return <PlateElement {...props} as={as as "div"} className={className} />;
  };
const leaf = (as: keyof HTMLElementTagNameMap, className = "") =>
  function Leaf(props: PlateLeafProps) {
    return <PlateLeaf {...props} as={as as "span"} className={className} />;
  };

function LinkElement(props: PlateElementProps<TElement & { url?: string; target?: string }>) {
  const { url, target } = props.element;
  return <PlateElement {...props} as="a" className="text-primary underline underline-offset-2"
    attributes={{ ...props.attributes, href: url, ...(target ? { target, rel: "noreferrer" } : {}) }} />;
}

function TableElement(props: PlateElementProps) {
  return <PlateElement {...props} as="table" className="my-2 w-full border-collapse text-sm">
    <tbody>{props.children}</tbody>
  </PlateElement>;
}

function CellElement(props: PlateElementProps<TElement & { colSpan?: number; rowSpan?: number }>) {
  const header = props.element.type === KEYS.th;
  return <PlateElement {...props} as={header ? "th" : "td"}
    className={header ? "border border-border bg-muted px-2 py-1 text-left font-semibold" : "border border-border px-2 py-1 align-top"}
    attributes={{ ...props.attributes, ...(props.element.colSpan ? { colSpan: props.element.colSpan } : {}), ...(props.element.rowSpan ? { rowSpan: props.element.rowSpan } : {}) }} />;
}

function CodeBlockElement(props: PlateElementProps) {
  return <PlateElement {...props} as="pre" className="my-2 overflow-x-auto rounded-md bg-muted p-3 font-mono text-sm">
    <code>{props.children}</code>
  </PlateElement>;
}

// A bare number in a width/height attribute means pixels.
const size = (value: unknown) => value === undefined || value === "" ? undefined : /^\d+(\.\d+)?$/.test(String(value)) ? `${value}px` : String(value);

function ImageElement(props: PlateElementProps<TElement & { url?: string; alt?: string; attrWidth?: string; attrHeight?: string; cssWidth?: string; cssHeight?: string }>) {
  const { url, alt, attrWidth, attrHeight, cssWidth, cssHeight } = props.element;
  const width = size(cssWidth ?? attrWidth), height = size(cssHeight ?? attrHeight);
  return <PlateElement {...props} className="my-2">
    <div contentEditable={false} className="inline-block max-w-full">
      <img src={url} alt={alt ?? ""} className="max-w-full rounded-md border border-border"
        style={{ ...(width ? { width } : {}), ...(height ? { height } : { maxHeight: "16rem" }) }} />
    </div>
    {props.children}
  </PlateElement>;
}

function HorizontalRuleElement(props: PlateElementProps) {
  return <PlateElement {...props} className="my-3">
    <div contentEditable={false}><hr className="border-t border-border" /></div>
    {props.children}
  </PlateElement>;
}

/** A template variable such as {{ user.name }}: one non-editable token. */
function PlaceholderElement(props: PlateElementProps<TElement & { name?: string }>) {
  const name = String(props.element.name ?? "");
  return <PlateElement {...props} as="span"
    className="mx-0.5 inline-block rounded bg-primary/10 px-1 font-mono text-[0.9em] text-primary"
    attributes={{ ...props.attributes, contentEditable: false, "aria-label": `${typedText("richtext.variable")} ${name}`, "data-placeholder-chip": name }}>
    {name}
    {props.children}
  </PlateElement>;
}

export const components = {
  [KEYS.p]: element("p", "my-1 min-h-6"),
  [KEYS.h1]: element("h1", "mb-2 mt-3 text-2xl font-semibold"),
  [KEYS.h2]: element("h2", "mb-2 mt-3 text-xl font-semibold"),
  [KEYS.h3]: element("h3", "mb-1 mt-2 text-lg font-semibold"),
  [KEYS.h4]: element("h4", "mb-1 mt-2 text-base font-semibold"),
  [KEYS.h5]: element("h5", "mb-1 mt-2 text-sm font-semibold"),
  [KEYS.h6]: element("h6", "mb-1 mt-2 text-xs font-semibold uppercase tracking-wide"),
  [KEYS.hr]: HorizontalRuleElement,
  [KEYS.blockquote]: element("blockquote", "my-2 border-l-4 border-border pl-3 text-muted-foreground"),
  [KEYS.codeBlock]: CodeBlockElement,
  [KEYS.codeLine]: element("div", ""),
  [KEYS.ulClassic]: element("ul", "my-1 list-disc pl-6"),
  [KEYS.olClassic]: element("ol", "my-1 list-decimal pl-6"),
  [KEYS.li]: element("li", ""),
  [KEYS.lic]: element("div", ""),
  [KEYS.link]: LinkElement,
  [KEYS.table]: TableElement,
  [KEYS.tr]: element("tr", ""),
  [KEYS.td]: CellElement,
  [KEYS.th]: CellElement,
  [KEYS.img]: ImageElement,
  [PLACEHOLDER]: PlaceholderElement,
  [KEYS.bold]: leaf("strong"),
  [KEYS.italic]: leaf("em"),
  [KEYS.underline]: leaf("u"),
  [KEYS.strikethrough]: leaf("s"),
  [KEYS.code]: leaf("code", "rounded bg-muted px-1 font-mono text-[0.9em]"),
};
