import { useState } from "react";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText as t, useCommonTranslation } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../../shared/ui/select";

const templates = [
  "general",
  "developer",
  "review",
  "operations",
  "workflow",
  "report",
];
const text = (key: string) => t(`agents:${key}`);

export function AgentPrompt({
  draft,
  readOnly,
  onPatch,
}: {
  draft: { system_prompt?: string };
  readOnly: boolean;
  onPatch: (patch: { system_prompt: string }) => void;
}) {
  useCommonTranslation();
  const [selected, setSelected] = useState("");
  const value = draft.system_prompt ?? "";
  const preview = selected ? text(`promptTemplate.${selected}.body`) : "";
  return (
    <div className="grid min-w-0 gap-3">
      {!readOnly && (
        <>
          <AutoField label={text("promptTemplates")}>
            <Select value={selected} onValueChange={setSelected}>
              <SelectTrigger
                className="w-full"
                aria-label={text("promptTemplates")}
              >
                <SelectValue placeholder={text("selectPromptTemplate")} />
              </SelectTrigger>
              <SelectContent>
                {templates.map((id) => (
                  <SelectItem key={id} value={id}>
                    {text(`promptTemplate.${id}.name`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </AutoField>
          {selected && (
            <div className="grid min-w-0 gap-3 rounded-md border bg-muted/30 p-3">
              <p className="whitespace-pre-wrap break-words text-sm text-muted-foreground">
                {preview}
              </p>
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  variant="outline"
                  onClick={() =>
                    onPatch({
                      system_prompt: value ? `${value}\n\n${preview}` : preview,
                    })
                  }
                >
                  {text(value ? "appendPromptTemplate" : "usePromptTemplate")}
                </Button>
                {value && (
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => onPatch({ system_prompt: preview })}
                  >
                    {text("replacePromptTemplate")}
                  </Button>
                )}
              </div>
            </div>
          )}
        </>
      )}
      <AutoField
        label={text("prompt")}
        name="system_prompt"
        help={!readOnly ? text("promptTemplateHelp") : undefined}
      >
        <textarea
          name="system_prompt"
          aria-label={text("prompt")}
          rows={10}
          readOnly={readOnly}
          value={value}
          onChange={(event) => onPatch({ system_prompt: event.target.value })}
          className="w-full min-w-0 resize-y rounded-md border border-input bg-background px-3 py-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
        />
      </AutoField>
    </div>
  );
}
