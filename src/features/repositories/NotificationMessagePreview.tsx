import type { Root } from "mdast";
import type { ReactNode } from "react";
import ReactMarkdown from "react-markdown";

// Only the native font tokens emitted by our delivery builder become spans.
// Raw HTML remains disabled; links and images follow the preview's allowlist.
const fontColors: Record<string, string> = {
  info: "#008000", comment: "#909399", warning: "#fa9d3b",
  blue: "#3370ff", green: "#34a853", red: "#d93025", purple: "#8e44ad",
};
type AstNode = { type: string; value?: string; children?: AstNode[]; data?: unknown };
function nativeColors() {
  return (tree: Root) => {
    function visit(node: AstNode) {
      const children = node.children;
      if (!children) return;
      for (let index = 0; index < children.length; index++) {
        const child = children[index]!;
        const match = child.type === "html" && /^<font color=["'](info|comment|warning|blue|green|red|purple)["']>$/.exec(child.value ?? "");
        if (match) {
          const end = children.findIndex((item, at) => at > index && item.type === "html" && item.value === "</font>");
          if (end >= 0) {
            const span: AstNode = { type: "notificationColor", children: children.slice(index + 1, end), data: { hName: "span", hProperties: { style: `color:${fontColors[match[1]!]}` } } };
            children.splice(index, end - index + 1, span); visit(span); continue;
          }
        }
        visit(child);
      }
    }
    visit(tree as unknown as AstNode);
  };
}
function safeUrl(url: string | undefined) {
  try { const value = new URL(url ?? ""); return ["http:", "https:"].includes(value.protocol) && !value.username && !value.password ? value.href : undefined; } catch { return undefined; }
}
function Markdown({ children }: { children: string }) {
  return <ReactMarkdown skipHtml remarkPlugins={[nativeColors]} components={{
    p: ({ children }) => <p className="m-0 whitespace-pre-wrap">{children}</p>,
    h3: ({ children }) => <h5 className="mb-2 text-base font-semibold">{children}</h5>,
    blockquote: ({ children }) => <blockquote className="m-0 border-l-2 border-[#d9d9d9] pl-3 leading-6">{children}</blockquote>,
    a: ({ href, children }) => safeUrl(href) ? <a href={safeUrl(href)} target="_blank" rel="noreferrer" className="text-[#2563eb] underline">{children}</a> : <>{children}</>,
    img: () => null,
  }}>{children}</ReactMarkdown>;
}
function object(value: unknown): Record<string, unknown> { return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {}; }
function text(value: unknown) { return typeof value === "string" ? value : ""; }
function chatMarkdown(value: string) {
  return value.replace(/<(https?:[^<>|]+)\|([^<>]+)>/g, (_, url: string, label: string) => `[${label.replace(/[\[\]\\]/g, "\\$&")}](${url.replace(/\(/g, "%28").replace(/\)/g, "%29")})`)
    .replace(/&lt;/g, "\\<").replace(/&gt;/g, "\\>").replace(/&amp;/g, "&");
}
function TelegramText({ value }: { value: string }) {
  const document = new DOMParser().parseFromString(value, "text/html");
  function render(node: Node, key: number): ReactNode {
    if (node.nodeType === Node.TEXT_NODE) return node.textContent;
    if (node instanceof Element && node.tagName === "A") {
      const href = safeUrl(node.getAttribute("href") ?? undefined);
      return href ? <a key={key} href={href} target="_blank" rel="noreferrer" className="text-[#2563eb] underline">{node.textContent}</a> : node.textContent;
    }
    return node.nodeType === Node.ELEMENT_NODE ? node.textContent : null;
  }
  return <div className="whitespace-pre-wrap">{[...document.body.childNodes].map(render)}</div>;
}

/** Render the exact native payload returned by the delivery builder. */
export function NotificationMessagePreview({ provider, payload }: { provider: string; payload: unknown }) {
  const data = object(payload);
  let content: ReactNode;
  if (provider === "WeCom" || provider === "DingTalk") {
    const markdown = object(data.markdown);
    content = <Markdown>{text(provider === "WeCom" ? markdown.content : markdown.text)}</Markdown>;
  } else if (provider === "Feishu" || provider === "Lark") {
    const card = object(data.card), header = object(card.header), elements = Array.isArray(card.elements) ? card.elements : [];
    const color = fontColors[text(header.template)] ?? "#1f2329";
    content = <div className="overflow-hidden rounded-md border border-[#dee0e3]">
      <h5 className="px-3 py-2 font-semibold" style={{ color, backgroundColor: `${color}14` }}>{text(object(header.title).content)}</h5>
      <div className="space-y-2 p-3">{elements.map((element, index) => <Markdown key={index}>{text(object(object(element).text).content)}</Markdown>)}</div>
    </div>;
  } else if (provider === "Telegram") {
    content = <TelegramText value={text(data.text)} />;
  } else if (provider === "Slack") {
    const attachments = Array.isArray(data.attachments) ? data.attachments : [];
    content = <div className="space-y-2"><Markdown>{chatMarkdown(text(data.text))}</Markdown>{attachments.map((entry, index) => {
      const attachment = object(entry), color = text(attachment.color);
      return <div key={index} className="border-l-4 pl-3" style={{ borderColor: /^#[0-9a-f]{6}$/i.test(color) ? color : "#d9d9d9" }}><Markdown>{chatMarkdown(text(attachment.text))}</Markdown></div>;
    })}</div>;
  } else if (provider === "Discord") {
    const embeds = Array.isArray(data.embeds) ? data.embeds : [];
    content = <div className="space-y-2">{data.content ? <Markdown>{text(data.content)}</Markdown> : null}{embeds.map((entry, index) => {
      const embed = object(entry), color = typeof embed.color === "number" ? `#${embed.color.toString(16).padStart(6, "0")}` : "#d9d9d9";
      return <div key={index} className="space-y-2 rounded border-l-4 bg-[#f5f6f7] p-3" style={{borderColor:color}}><h5 className="font-semibold">{text(embed.title)}</h5><Markdown>{text(embed.description)}</Markdown></div>;
    })}</div>;
  } else if (provider === "Google Chat") {
    content = <Markdown>{chatMarkdown(text(data.text))}</Markdown>;
  } else {
    content = <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all text-xs">{JSON.stringify(payload, null, 2)}</pre>;
  }
  return <div data-notification-message={provider} className="min-w-0 break-words rounded-md bg-white p-3 text-sm leading-6 text-[#1f2329]">{content}</div>;
}
