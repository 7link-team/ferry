import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

export default function MarkdownValuePreview({ value }: { value: string }) {
  return <div aria-label="Markdown 预览" className="mt-2 space-y-3 overflow-x-auto rounded-md border p-4 text-sm [&_h1]:text-2xl [&_h2]:text-xl [&_h3]:text-lg [&_h1]:font-semibold [&_h2]:font-semibold [&_h3]:font-semibold [&_a]:text-primary [&_a]:underline [&_ul]:list-disc [&_ol]:list-decimal [&_ul]:pl-6 [&_ol]:pl-6 [&_pre]:overflow-auto [&_pre]:rounded [&_pre]:bg-muted [&_pre]:p-3 [&_table]:w-full [&_td]:border [&_th]:border [&_td]:p-2 [&_th]:p-2 [&_img]:max-w-full"><ReactMarkdown remarkPlugins={[remarkGfm]}>{value}</ReactMarkdown></div>;
}
