import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  codeInsightForm,
  codeInsightList,
} from "../../features/code-insights/config";
import { insightText } from "../../shared/i18n/i18n";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../shared/ui/tabs";
import { ResourceForm } from "../resources/ResourceForm";
import { ResourceTable } from "../resources/ResourceTable";
import { href, useQueryValue } from "../router";
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
  const [tab, setTab] = useQueryValue("tab", "dashboard");
  const form = useMemo(() => codeInsightForm(workspace), [workspace]);
  const summary = useMemo(
    () => ({
      ...form,
      sections: form.sections.filter(
        (section) =>
          section.title === "基本信息" || section.title === "同步覆盖",
      ),
      readOnly: () => true,
    }),
    [form],
  );
  const aliases = useMemo(
    () => ({
      ...form,
      description: "",
      incompleteMessage: "每条作者映射须填写姓名或邮箱，以及显示昵称。",
      incompleteMessageKey:
        "insight-domain:每条作者映射须填写姓名或邮箱，以及显示昵称。",
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
  return (
    <Tabs
      value={tab === "settings" || tab === "authors" ? tab : "dashboard"}
      onValueChange={setTab}
      className="flex min-h-0 min-w-0 flex-1 flex-col gap-4"
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
        <div className="shrink-0">
          <ResourceForm
            config={summary}
            workspace={workspace}
            permissions={permissions}
            showViewNavigation={false}
          />
        </div>
        <RepositoryAnalysis
          workspace={workspace}
          connectionId={insightId}
          scope="insight"
          permissions={permissions}
        />
      </TabsContent>
      <TabsContent value="settings" className="min-h-0 flex-1 overflow-auto">
        <ResourceForm
          config={form}
          workspace={workspace}
          permissions={permissions}
        />
      </TabsContent>
      <TabsContent value="authors" className="min-h-0 flex-1 overflow-auto">
        <ResourceForm
          config={aliases}
          workspace={workspace}
          permissions={permissions}
          editInPlace
          showViewNavigation={false}
          onCancel={() => setTab("dashboard")}
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
              { group: "build", item: "code-insights", mode: "list", id: "" },
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
