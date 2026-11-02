import { typedText } from "../i18n";
import type { FormConfig, ListConfig } from "./dsl";
import { deleteResource } from "../resource";
import { AutoField } from "./auto-form";
import { initialValue, TypedSchemaEditor, TypedValueEditor, VALUE_TYPES, valueTypeOptions, type TypedValueRow } from "./TypedValueEditor";

type Resource = "dictionaries" | "settings";
export function typedResourceList(workspace: string, resource: Resource): ListConfig<TypedValueRow> {
  return {
    resource, rowKey: row => row.id,
    filters: [{ key: "q", label: "标识 / 名称", labelKey: "typed-domain:search", type: "text" }, { key: "category", label: "分类", labelKey: "typed-domain:category", type: "text" }, { key: "value_type", label: "值类型", labelKey: "typed-domain:valueType", type: "select", options: valueTypeOptions }],
    columns: [
      { key: "category", label: "分类", labelKey: "typed-domain:category", sortable: true }, { key: "key", label: "标识", labelKey: "typed-domain:key", sortable: true },
      { key: "name", label: "名称", labelKey: "typed-domain:name", sortable: true }, { key: "description", label: "说明", labelKey: "typed-domain:description" },
      { key: "value_type", label: "值类型", labelKey: "typed-domain:valueType", render: row => VALUE_TYPES.some(([key]) => key === row.value_type) ? typedText(`type.${row.value_type}` as Parameters<typeof typedText>[0]) : row.value_type },
      { key: "value_summary", label: "当前值", labelKey: "typed-domain:currentValue" },
      ...(resource === "settings" ? [{ key: "default", label: "默认值", labelKey: "typed-domain:defaultValue", render: (row: TypedValueRow) => row.builtin ? String(row.default ?? "—") : "—" }] : []),
      { key: "builtin", label: "来源", labelKey: "typed-domain:source", render: row => row.builtin ? typedText("builtin") : typedText("custom") },
    ],
    actions: [
      { label: "查看", labelKey: "typed-domain:view", to: row => ({ id: row.id, mode: "view" }) },
      { label: "编辑", labelKey: "typed-domain:edit", permission: `${resource}.update`, to: row => ({ id: row.id, mode: "edit" }) },
      { label: "复制", labelKey: "typed-domain:copy", permission: `${resource}.create`, to: () => ({ mode: "new" }), query: row => ({ copy: row.id }) },
      { label: "删除", labelKey: "typed-domain:delete", permission: `${resource}.delete`, when: row => !row.builtin, destructive: true, action: row => deleteResource(workspace, resource, row.id) },
    ],
    toolbar: { import: true, export: true },
  };
}
export function typedResourceForm(resource: Resource): FormConfig<TypedValueRow> {
  const isSetting = resource === "settings";
  const type = isSetting ? "text" : "selection";
  return {
    resource, heading: row => row.name,
    blank: { key: "", name: "", description: "", category: "general", value_type: type, value_json: initialValue(type), schema_json: "{}", ...(isSetting ? {} : { items: "[]" }) },
    copy: row => ({ key: `${row.key}-copy`, name: typedText("copyName", { name: row.name }), description: row.description, category: row.category, value_type: row.value_type, value_json: row.value_json, schema_json: row.schema_json, items: row.items, value: row.value, builtin: false }),
    sections: [
      { title: "基础信息", titleKey: "typed-domain:basic", fields: [
        { key: "category", label: "分类", labelKey: "typed-domain:category", type: "text", required: true, disabled: row => !!row.builtin && isSetting },
        { key: "key", label: "标识", labelKey: "typed-domain:key", type: "text", required: true, immutable: true },
        { key: "name", label: "名称", labelKey: "typed-domain:name", type: "text", required: true, disabled: row => !!row.builtin && isSetting },
        { key: "value_type", label: "值类型", labelKey: "typed-domain:valueType", type: "custom", disabled: row => !!row.builtin, render: ({ draft, readOnly, onPatch }) => <AutoField label={typedText("valueType")}><select aria-label={typedText("valueType")} className="h-9 w-full rounded-md border bg-background px-3 text-sm" value={draft.value_type ?? type} disabled={readOnly} onChange={event => { const next = event.target.value; onPatch({ value_type: next, value_json: initialValue(next), schema_json: "{}", items: "[]", ...(next === "num" ? { value: 0 } : {}) }); }}>{valueTypeOptions.map(option => <option key={option.value} value={option.value}>{typedText(`type.${option.value}`)}</option>)}</select></AutoField> },
        { key: "description", label: "说明", labelKey: "typed-domain:description", type: "textarea", span: 2, disabled: row => !!row.builtin && isSetting },
      ] },
      { title: "值约束", titleKey: "typed-domain:constraints", fields: [{ key: "schema_json", label: "值约束", labelKey: "typed-domain:constraints", type: "custom", span: 2, disabled: row => !!row.builtin, render: props => <TypedSchemaEditor {...props} resource={resource} /> }] },
      { title: "值", titleKey: "typed-domain:value", fields: [{ key: "value_json", label: "值", labelKey: "typed-domain:value", type: "custom", span: 2, render: props => <TypedValueEditor {...props} resource={resource} /> }] },
    ],
  };
}
