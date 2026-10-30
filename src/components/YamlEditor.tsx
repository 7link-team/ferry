import { useRef } from "react";
import { Upload } from "lucide-react";
import { Button } from "./ui/button";
import { useTheme } from "../theme";
import Editor, { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
import { configureMonacoYaml } from "monaco-yaml";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker.js?worker";
import YamlWorker from "./yaml.worker?worker";

globalThis.MonacoEnvironment = {
  getWorker(_moduleId, label) {
    return label === "yaml" ? new YamlWorker() : new EditorWorker();
  },
};
loader.config({ monaco });
configureMonacoYaml(monaco, { enableSchemaRequest: false });

export function YamlEditor({ value, readOnly, fillHeight, onChange }: {
  value: string;
  readOnly: boolean;
  fillHeight: boolean;
  onChange: (value: string) => void;
}) {
  const theme = useTheme();
  const fileInput = useRef<HTMLInputElement>(null);
  return <div className={fillHeight ? "flex min-h-0 flex-1 flex-col gap-2" : "space-y-2"}>
    <div className="flex items-center justify-between gap-2"><span className="text-sm font-medium">GitHub Actions YAML</span>{!readOnly && <><input ref={fileInput} className="hidden" type="file" accept=".yml,.yaml,text/yaml" aria-label="选择 Actions YAML 文件" onChange={async event => { const file = event.target.files?.[0]; if (file) onChange(await file.text()); event.target.value = ""; }} /><Button type="button" variant="outline" size="sm" onClick={() => fileInput.current?.click()}><Upload className="size-4" />导入 YAML</Button></>}</div>
    <div className={fillHeight ? "min-h-[440px] flex-1 overflow-hidden rounded-md border border-input" : "h-[520px] min-h-[440px] overflow-hidden rounded-md border border-input"}>
      <Editor
        path="file:///pipeline.yaml"
        language="yaml"
        theme={theme.resolved === "dark" ? "vs-dark" : "vs"}
        value={value}
        onChange={next => onChange(next ?? "")}
        // Monaco 0.54 word highlighting leaves a rejected debounce promise on
        // disposal when switching modes. YAML validation/completion stays enabled.
        options={{ occurrencesHighlight: "off", ariaLabel: "GitHub Actions YAML", readOnly, automaticLayout: true, minimap: { enabled: false }, scrollBeyondLastLine: false, fontSize: 13, tabSize: 2, wordWrap: "on" }}
      />
    </div>
  </div>;
}
