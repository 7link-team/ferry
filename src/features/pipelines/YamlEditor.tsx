import { Upload } from "lucide-react";
import { useRef } from "react";
import { CodeEditor } from "../../shared/editor/CodeEditor";
import { Button } from "../../shared/ui/button";

export function YamlEditor({ value, readOnly, fillHeight, onChange }: {
  value: string;
  readOnly: boolean;
  fillHeight: boolean;
  onChange: (value: string) => void;
}) {
  const fileInput = useRef<HTMLInputElement>(null);
  return <div className={fillHeight ? "flex min-h-0 flex-1 flex-col gap-2" : "space-y-2"}>
    <div className="flex items-center justify-between gap-2"><span className="text-sm font-medium">GitHub Actions YAML</span>{!readOnly && <><input ref={fileInput} className="hidden" type="file" accept=".yml,.yaml,text/yaml" aria-label="选择 Actions YAML 文件" onChange={async event => { const file = event.target.files?.[0]; if (file) onChange(await file.text()); event.target.value = ""; }} /><Button type="button" variant="outline" size="sm" onClick={() => fileInput.current?.click()}><Upload className="size-4" />导入 YAML</Button></>}</div>
    <div className={fillHeight ? "min-h-[440px] flex-1 overflow-hidden rounded-md border border-input" : "h-[520px] min-h-[440px] overflow-hidden rounded-md border border-input"}>
      <CodeEditor
        path="file:///pipeline.yaml"
        language="yaml"
        label="GitHub Actions YAML"
        value={value}
        onChange={onChange}
        readOnly={readOnly}
      />
    </div>
  </div>;
}
