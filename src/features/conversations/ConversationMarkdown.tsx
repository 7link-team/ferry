import { Children, createContext, isValidElement, useContext, useEffect, useId, useRef, useState } from "react";
import type { ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import type { Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import DOMPurify from "dompurify";
import { useTranslation } from "react-i18next";
import { createThemeColorReader, useTheme } from "../../shared/theme/theme";
import { ConversationReport } from "./ConversationReport";
import { CollapsibleMessage } from "./CollapsibleMessage";
import { ConversationResourceLink, conversationResource } from "./ConversationResourceLink";
const MarkdownEndpoint = createContext<string | undefined>(undefined);
function MessageLink({ href, children }: { href?: string | undefined; children?: ReactNode }) {
  const endpoint = useContext(MarkdownEndpoint);
  const name = Children.toArray(children).filter(v => typeof v === "string").join("");
  const prefix = `/api${endpoint}/files/`;
  if (endpoint && href?.startsWith(prefix) && /^\d+$/.test(href.slice(prefix.length)) && /\.md$/i.test(name)) {
    return <ConversationReport href={href} name={name} renderContent={text => <ConversationMarkdown endpoint={endpoint}>{text}</ConversationMarkdown>} />;
  }
  const resource = conversationResource(href, endpoint);
  if (resource) return <ConversationResourceLink resource={resource}>{children}</ConversationResourceLink>;
  return <a href={href} target="_blank" rel="noopener noreferrer">{children}</a>;
}

// Mermaid's initialization is global. Keep configuration paired with its own render.
let diagramQueue: Promise<void> = Promise.resolve();

function MermaidDiagram({ source }: { source: string }) {
  const { t } = useTranslation("conversations");
  const { palette, resolved, customLight, customDark } = useTheme();
  const themeKey = JSON.stringify([palette, resolved, customLight, customDark]);
  const id = "conversation-diagram-" + useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const renderVersion = useRef(0);
  const figure = useRef<HTMLElement>(null);
  const [result, setResult] = useState({ source: "", svg: "", error: false });
  useEffect(() => {
    let active = true;
    // Streaming answers can contain incomplete syntax. Wait for a short pause.
    const timer = setTimeout(async () => {
      try {
        if (source.length > 20_000 || /%%\{|^\s*---/.test(source)) throw new Error("Unsupported diagram configuration");
        const { default: mermaid } = await import("mermaid");
        if (!active || !figure.current) return;
        const read = createThemeColorReader(figure.current);
        const dark = resolved === "dark";
        const background = read("--card", dark ? "#171717" : "#ffffff");
        const text = read("--foreground", dark ? "#fafafa" : "#171717");
        const node = read("--muted", dark ? "#262626" : "#f5f5f5");
        const accent = read("--accent", node);
        const accentText = read("--accent-foreground", text);
        const border = read("--primary", text);
        const themeVariables = {
          darkMode: dark, background, textColor: text, lineColor: text,
          primaryColor: node, primaryTextColor: text, primaryBorderColor: border,
          secondaryColor: accent, secondaryTextColor: accentText, secondaryBorderColor: border,
          tertiaryColor: background, tertiaryTextColor: text, tertiaryBorderColor: border,
          mainBkg: node, nodeBorder: border, clusterBkg: node, clusterBorder: border,
          titleColor: text, edgeLabelBackground: background,
          actorBkg: node, actorTextColor: text, actorBorder: border,
          signalColor: text, signalTextColor: text, labelBoxBkgColor: background,
          labelTextColor: text, noteBkgColor: node, noteTextColor: text, noteBorderColor: border,
          activationBkgColor: accent, activationBorderColor: border,
          fontFamily: getComputedStyle(figure.current).fontFamily,
        };
        const renderId = `${id}-${++renderVersion.current}`;
        const job = diagramQueue.then(async () => {
          if (!active) return "";
          mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme: "base", themeVariables, htmlLabels: false, suppressErrorRendering: true, maxTextSize: 20_000, maxEdges: 500, flowchart: { htmlLabels: false } });
          return (await mermaid.render(renderId, source)).svg;
        });
        diagramQueue = job.then(() => undefined, () => undefined);
        const svg = await job;
        const safe = DOMPurify.sanitize(svg, { USE_PROFILES: { svg: true, svgFilters: true }, FORBID_TAGS: ["foreignObject", "script", "image", "use", "a"], FORBID_ATTR: ["href", "xlink:href"] });
        if (active) setResult({ source, svg: safe, error: false });
      } catch {
        if (active) setResult({ source, svg: "", error: true });
      }
    }, result.source === source ? 0 : 300);
    return () => { active = false; clearTimeout(timer); };
  }, [id, source, themeKey, resolved]);
  const current = result.source === source;
  return (
    <figure ref={figure} aria-label={t("diagram")} className="conversation-diagram my-4 min-w-0 max-w-full overflow-hidden rounded-lg border bg-card p-3 text-card-foreground">
      {!current ? <div role="status" aria-label={t("diagramLoading")} className="h-24 animate-pulse rounded bg-muted motion-reduce:animate-none" /> : result.error ? <p className="text-sm text-muted-foreground">{t("diagramError")}</p> : <div className="max-h-[32rem] max-w-full overflow-auto [&_svg]:h-auto [&_svg]:min-w-[40rem] [&_svg]:max-w-full" dangerouslySetInnerHTML={{ __html: result.svg }} />}
      <details key={String(current && result.error)} open={current && result.error ? true : undefined} className="mt-2 text-xs text-muted-foreground">
        <summary className="cursor-pointer py-2 focus-visible:outline focus-visible:outline-ring">{t("diagramSource")}</summary>
        <pre className="max-h-64 overflow-auto whitespace-pre p-2"><code>{source}</code></pre>
      </details>
    </figure>
  );
}

// Stable renderers retain diagram state and table scroll position during polling.
const markdownComponents: Components = {
    a: MessageLink,
    img: ({ alt }) => <span>{alt}</span>,
    pre: ({ children }) => {
      const code = Children.toArray(children)[0];
      if (isValidElement<{ className?: string; children?: ReactNode }>(code) && code.props.className === "language-mermaid") {
        return <MermaidDiagram source={String(code.props.children ?? "").replace(/\n$/, "")} />;
      }
      return <pre>{children}</pre>;
    },
};

export function ConversationMarkdown({ children, endpoint }: { children: string; endpoint?: string | undefined }) {
  return <MarkdownEndpoint.Provider value={endpoint}><div className="conversation-markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>{children}</ReactMarkdown></div></MarkdownEndpoint.Provider>;
}

/** Keep deliverable cards reachable even when the accompanying answer is folded. */
export function ConversationMessage({ children, endpoint, streaming }: { children: string; endpoint: string; streaming: boolean }) {
  const reports: { name: string; href: string }[] = [];
  const prefix = `/api${endpoint}/files/`;
  const text = children.replace(/\[([^\]\n]+\.md)\]\(([^)\s]+)\)/gi, (original, name: string, href: string) => {
    if (!href.startsWith(prefix) || !/^\d+$/.test(href.slice(prefix.length))) return original;
    if (!reports.some(item => item.href === href)) reports.push({name,href});
    return "";
  });
  return <><CollapsibleMessage streaming={streaming}><ConversationMarkdown endpoint={endpoint}>{text}</ConversationMarkdown></CollapsibleMessage>{reports.map(report => <ConversationReport key={report.href} {...report} renderContent={value => <ConversationMarkdown endpoint={endpoint}>{value}</ConversationMarkdown>} />)}</>;
}
