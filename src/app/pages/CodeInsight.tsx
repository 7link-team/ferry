import { useCallback, useMemo, useState } from "react";
import { CodeInsightStatus } from "../../features/code-insights/CodeInsightStatus";
import { useTranslation } from "react-i18next";
import {
  codeInsightForm,
  codeInsightList,
} from "../../features/code-insights/config";
import { insightText } from "../../shared/i18n/i18n";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../shared/ui/tabs";
import { ResourceForm } from "../resources/ResourceForm";
import { ResourceTable } from "../resources/ResourceTable";
import { href, useQueryValue, useRoute } from "../router";
import { RepositoryAnalysis } from "./RepositoryAnalysis";
export * from "../../features/code-insights/config";

export function CodeInsightDetail({
  workspace,
  insightId,
  permissions,
}: {
  workspace: string;
  insightId: string;
  permissions: Set<string>;
}) {
  useTranslation("insight-domain");
  const route = useRoute();
  const [tab, setTab] = useQueryValue("tab", "dashboard");
  const form = useMemo(() => codeInsightForm(workspace), [workspace]);
  const [published, setPublished] = useState(0);
  const onPublished = useCallback(() => setPublished(value => value + 1), []);
  const aliases = useMemo(
    () => ({
      ...form,
      description: "",
      sections: [
        {
          title: "作者昵称映射",
          titleKey: "insight-domain:作者昵称映射",
          fields: [
            {
              key: "aliases",
              label: "作者昵称映射",
              labelKey: "insight-domain:作者昵称映射",
              type: "author-aliases" as const,
              span: 2 as const,
            },
          ],
        },
      ],
      readOnly: () => !permissions.has("code-insights.update"),
      viewActions: [],
    }),
    [form, permissions],
  );
  const editHref = href({ ...route, mode: "edit" }, { tab, return_to: location.pathname + location.search });
  if (route.mode === "edit") return <div className="crud-config-page flex min-h-0 min-w-0 flex-1 flex-col"><ResourceForm config={tab === "authors" ? aliases : form} workspace={workspace} permissions={permissions} /></div>;
  return (
    <Tabs
      value={tab === "settings" || tab === "authors" ? tab : "dashboard"}
      onValueChange={setTab}
      className="crud-config-page flex min-h-0 min-w-0 flex-1 flex-col gap-4"
    >
      <div className="min-w-0 overflow-x-auto border-b pb-2">
        <TabsList aria-label={insightText("代码洞察详情")}>
          <TabsTrigger value="dashboard">{insightText("分析看板")}</TabsTrigger>
          <TabsTrigger value="settings">
            {insightText("配置与同步")}
          </TabsTrigger>
          <TabsTrigger value="authors">
            {insightText("作者昵称映射")}
          </TabsTrigger>
        </TabsList>
      </div>
      <TabsContent
        value="dashboard"
        className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto"
      >
        <CodeInsightStatus workspace={workspace} insightId={insightId} permissions={permissions} onConfigure={() => setTab("settings")} onPublished={onPublished} />
        <RepositoryAnalysis
          refreshVersion={published}
          workspace={workspace}
          connectionId={insightId}
          scope="insight"
          permissions={permissions}
        />
      </TabsContent>
      <TabsContent value="settings" className="min-h-0 flex-1 overflow-auto">
        <ResourceForm
          config={form}
          editHref={editHref}
          workspace={workspace}
          permissions={permissions}
        />
      </TabsContent>
      <TabsContent value="authors" className="min-h-0 flex-1 overflow-auto">
        <ResourceForm
          config={aliases}
          workspace={workspace}
          permissions={permissions}
          editHref={editHref}
        />
      </TabsContent>
    </Tabs>
  );
}

export function ConnectionInsights({
  workspace,
  connectionId,
  permissions,
}: {
  workspace: string;
  connectionId: string;
  permissions: Set<string>;
}) {
  useTranslation("insight-domain");
  const list = useMemo(
    () => codeInsightList(workspace, connectionId),
    [workspace, connectionId],
  );
  return (
    <section
      aria-label={insightText("相关代码洞察")}
      className="flex min-h-0 flex-1 flex-col gap-3"
    >
      <p className="text-sm text-muted-foreground">
        {insightText(
          "在代码洞察中选择本连接的仓库，独立配置作者映射并查看分析看板。",
        )}
      </p>
      {permissions.has("code-insights.read") ? (
        <>
          <a
            className="text-sm text-primary underline underline-offset-4"
            href={href(
              { group: "repository", item: "code-insights", mode: "list", id: "" },
              {},
            )}
          >
            {insightText("管理代码洞察")}
          </a>
          <ResourceTable
            config={list}
            workspace={workspace}
            permissions={permissions}
          />
        </>
      ) : (
        <p className="text-sm text-muted-foreground">
          {insightText("查看相关洞察需要代码洞察读取权限。")}
        </p>
      )}
    </section>
  );
}
