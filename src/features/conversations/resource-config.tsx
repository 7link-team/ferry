import { commonText } from "../../shared/i18n/i18n";
import {
  form,
  list,
  namedOptions,
  cronDate,
  type AnyRow,
  type ResourceConfigContext,
} from "../../shared/resources/config";
import type { FormConfig, ListConfig, Option } from "../../shared/resources/types";
import { listResource } from "../../shared/api/resource";
export const conversationText = (key: string) =>
  commonText(`conversations:${key}`);
const key = (value: string) => `conversations:${value}`;
/** The server applies Agent visibility and use permissions for API and CLI models. */
export const conversationAgentOptions = (workspace: string, conditions: Record<string, string>) =>
  async (): Promise<Option[]> => {
    const items: Option[] = [];
    for (let page = 1; ; page += 1) {
      const result = await listResource<{ id: string; name: string; runtime?: string }>(
        workspace, "ai-agents", { ...conditions, page, page_size: 200 });
      items.push(...result.items.map(row => ({ value: row.id, label: row.name })));
      if (!result.items.length || page * result.page_size >= result.total) return items;
    }
  };
export function conversationLists(
  _: ResourceConfigContext,
): Record<string, ListConfig<AnyRow>> {
  return {
    conversations: list<AnyRow>({
      resource: "conversations",
      rowKey: (r) => r.id,
      filters: [
        { key: "q", label: "Title", labelKey: key("title"), type: "text" },
      ],
      columns: [
        {
          key: "title",
          label: "Title",
          labelKey: key("title"),
          sortable: true,
        },
        { key: "agent_name", label: "Agent" },
        {
          key: "state",
          label: "State",
          labelKey: "agents:state",
          render: (r) => conversationText(r.state),
        },
        {
          key: "updated_at",
          label: "Updated",
          labelKey: key("updated"),
          render: (r) => cronDate(r.updated_at),
        },
      ],
      actions: [
        {
          label: "View",
          labelKey: "common-domain:common.view",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        {
          label: "Edit",
          labelKey: "common-domain:common.edit",
          permission: "conversations.update",
          to: (r) => ({ id: r.id, mode: "edit" }),
        },
      ],
      toolbar: { import: false, export: false },
      empty: "",
      emptyKey: key("empty"),
    }),
  };
}
export function conversationForms({
  workspace,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    conversations: form<AnyRow>({
      resource: "conversations",
      heading: (r) => r.title,
      blank: {
        title: "",
        kind: "private",
        member_ids: [],
        agent_ids: [],
        agent_id: "",
        state: "active",
      },
      sections: [
        {
          fields: [
            {
              key: "kind",
              label: "Conversation type",
              labelKey: key("kind"),
              type: "select",
              required: true,
              when: (r) => !r.id,
              resetOnChange: ["agent_id", "member_ids", "agent_ids"],
              options: ["private", "group"].map((value) => ({
                value,
                label: value,
                labelKey: key(value === "private" ? "direct" : "group"),
              })),
            },
            {
              key: "title",
              label: "Title",
              labelKey: key("title"),
              type: "text",
            },
            {
              key: "agent_id",
              label: "Agent",
              labelKey: key("agent"),
              type: "select",
              required: true,
              immutable: true,
              when: (r) => r.kind !== "group",
              options: conversationAgentOptions(workspace, { for_use: "true" }),
              display: (r) => r.agent_name,
            },
            {
              key: "member_ids",
              label: "People",
              labelKey: key("people"),
              type: "multiselect",
              searchable: true,
              when: (r) => !r.id && r.kind === "group",
              options: namedOptions(workspace, "conversation-participants"),
            },
            {
              key: "agent_ids",
              label: "Agents",
              labelKey: "agents:title",
              type: "multiselect",
              searchable: true,
              required: true,
              when: (r) => !r.id && r.kind === "group",
              options: conversationAgentOptions(workspace, {
                for_use: "true",
                visibility: "workspace",
              }),
            },
            {
              key: "state",
              label: "State",
              labelKey: "agents:state",
              type: "select",
              when: (r) => !!r.id,
              disabled: (r) => r.state === "interrupted",
              options: [
                { value: "active", label: "Active", labelKey: key("active") },
                {
                  value: "interrupted",
                  label: "Interrupted",
                  labelKey: key("interrupted"),
                  disabled: true,
                },
                {
                  value: "archived",
                  label: "Archived",
                  labelKey: key("archived"),
                },
              ],
            },
          ],
        },
      ],
    }),
  };
}
