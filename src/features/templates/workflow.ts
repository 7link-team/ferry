import { commonText } from "../../shared/i18n/i18n";
import { parseDocument } from "yaml";

/** Templates persist an envelope; the shared workflow editor edits its contents. */
export function templateWorkflow(content: string): Record<string, unknown> {
  const document = parseDocument(content);
  if (document.errors.length) throw document.errors[0];
  const value: unknown = document.toJS();
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error(commonText("resource-domain:template.invalidActions"));
  const envelope = value as Record<string, unknown>;
  if (!["github-actions", "actions"].includes(String(envelope.engine)) || typeof envelope.workflow !== "string")
    throw new Error(commonText("resource-domain:template.invalidActions"));
  return envelope;
}

export function patchTemplateWorkflow(envelope: Record<string, unknown>, patch: Record<string, unknown>) {
  const next = { ...envelope };
  for (const key of ["executor_bindings", "checkout_bindings", "graph_layout"])
    if (key in patch) next[key] = patch[key];
  if ("definition" in patch) next.workflow = patch.definition;
  return { content: JSON.stringify(next, null, 2) };
}
