import type { FormConfig } from "../../shared/resources/types";
import { applicationConfigurationField } from "../templates/ApplicationConfigurationFields";

/** Shared configuration hierarchy for marketplace packages, templates and instances. */
export function applicationSections<T>(sections: FormConfig<T>["sections"], workspace: string): FormConfig<T>["sections"] {
  const fields = sections.flatMap(section => section.fields).map(field => applicationConfigurationField(field, workspace));
  const groups = [
    ["basics", "common-domain:common.basic", ["name", "description", "category", "package_version", "version", "updated_at", "readme"]],
    ["target", "resource-domain:deployment.targetSection", ["mode", "target_kind", "target_ref", "workdir"]],
    ["definition", "resource-domain:template.definition", ["content", "manifest"]],
    ["parameters", "resource-domain:template.parameters", ["parameters", "values"]],
    ["files", "resource-domain:market.files", ["files"]],
    ["runtime", "resource-domain:application.runtime", ["runtime"]],
  ] as const;
  return groups.flatMap(([group, groupKey, keys]) => {
    const selected = fields.filter(field => (keys as readonly string[]).includes(field.key));
    return selected.length ? [{ group, groupKey, title: group, titleKey: groupKey, fields: selected }] : [];
  });
}
