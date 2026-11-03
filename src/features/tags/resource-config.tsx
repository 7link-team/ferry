import { TagBadge } from "./TagBadge";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText } from "../../shared/i18n/i18n";
import { deleteResource } from "../../shared/api/resource";
import type { ResourceConfigContext } from "../../shared/resources/config";
import {
  type AnyRow,
  form,
  list,
  namedOptions,
} from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";

export interface Tag {
  id: string;
  name: string;
  color: string;
  version: number;
}

export const tagOptions = (workspace: string) =>
  namedOptions<Tag>(workspace, "tags", tag => <TagBadge color={tag.color}>{tag.name}</TagBadge>);

export function tagsLists({
  workspace,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    tags: list({
      resource: "tags",
      rowKey: (row: Tag) => row.id,
      filters: [
        {
          key: "q",
          label: "名称",
          labelKey: "common-domain:common.name",
          type: "text",
          placeholder: "输入关键字",
          placeholderKey: "resource-domain:resource.keyword",
        },
      ],
      columns: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name" },
        {
          key: "color",
          label: "颜色",
          labelKey: "resource-domain:tag.color",
          width: 120,
          render: (row: Tag) =>
            row.color ? (
              <span className="inline-flex items-center gap-2">
                <span
                  className="inline-block size-3 rounded-full border border-border"
                  style={{ background: row.color }}
                />
                {row.color}
              </span>
            ) : (
              "—"
            ),
        },
      ],
      actions: [
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "tags.update",
          to: (row: Tag) => ({ id: row.id, mode: "edit" }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "tags.delete",
          destructive: true,
          action: (row: Tag) => deleteResource(workspace, "tags", row.id),
        },
      ],
      empty: "还没有标签。",
      emptyKey: "resource-domain:tag.empty",
    }),
  };
}

export function tagsForms({}: ResourceConfigContext): Record<
  string,
  FormConfig<AnyRow>
> {
  return {
    tags: form({
      resource: "tags",
      heading: (row: Tag) => row.name,
      blank: { name: "", color: "#4f9cf9" },
      sections: [
        {
          fields: [
            {
              key: "name",
              label: "名称",
              labelKey: "common-domain:common.name",
              type: "text",
              required: true,
            },
            {
              key: "color",
              label: "颜色",
              labelKey: "resource-domain:tag.color",
              type: "custom",
              render: ({ draft, readOnly, onPatch }) => <AutoField label={commonText("resource-domain:tag.color")}><div className="flex items-center gap-3"><input type="color" aria-label={commonText("resource-domain:tag.color")} className="h-10 w-14 cursor-pointer rounded-md border border-input bg-background p-1 disabled:cursor-default" value={/^#[\da-f]{6}$/i.test(draft.color ?? "") ? draft.color! : "#4f9cf9"} disabled={readOnly} onChange={event => onPatch({ color: event.target.value })} /><TagBadge color={draft.color}>{draft.name || commonText("common.name")}</TagBadge></div></AutoField>,

            },
          ],
        },
      ],
    }),
  };
}
