import { useEffect, useState } from "react";
import { GitBranch, Plus, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { errorText } from "../../shared/api/api";
import { listResource } from "../../shared/api/resource";
import { AutoField } from "../../shared/forms/auto-form";
import { insightText } from "../../shared/i18n/i18n";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import {
  ConnectionRepositoryFields,
  useConnectionRepositories,
} from "../repositories/ConnectionRepositoryFields";

export interface InsightRepository {
  connection_id: string;
  repository: string;
  pipeline_id: string;
  all_repositories?: boolean;
  connection_name?: string;
  pipeline_name?: string;
  provider?: string;
}
type Props = {
  navigation: Navigation;
  workspace: string;
  value: unknown;
  readOnly: boolean;
  permissions: Set<string>;
  onChange: (value: unknown) => void;
  onValidityChange: (blocked: boolean) => void;
};
const selectClass =
  "h-10 w-full rounded-md border border-input bg-background px-3 text-sm";

/** One shared form field manages memberships; connection browsing is reused from checkout. */
export function CodeInsightRepositories({
  navigation,
  workspace,
  value,
  readOnly,
  permissions,
  onChange,
  onValidityChange,
  enabled = true,
}: Props & { enabled?: boolean }) {
  useTranslation("insight-domain");
  const selected = Array.isArray(value) ? (value as InsightRepository[]) : [];
  const canSelect = permissions.has("repositories.read");
  const canPipeline =
    permissions.has("pipelines.read") && permissions.has("pipelines.run");
  const [choice, setChoice] = useState({ connection_id: "", url: "" });
  const data = useConnectionRepositories(
    workspace,
    choice.connection_id,
    !readOnly && canSelect,
  );
  const [pipelines, setPipelines] = useState<{ id: string; name: string }[]>(
    [],
  );
  const [pipelineError, setPipelineError] = useState("");
  const [pipelineLoading, setPipelineLoading] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    if (readOnly || !canPipeline) return;
    const controller = new AbortController();
    setPipelineLoading(true);
    setPipelineError("");
    void (async () => {
      const all: { id: string; name: string }[] = [];
      for (let page = 1; ; page++) {
        const result = await listResource<{ id: string; name: string }>(
          workspace,
          "pipelines",
          { page, page_size: 100 },
          controller.signal,
        );
        all.push(...result.items);
        if (!result.items.length || page * 100 >= result.total) break;
      }
      if (!controller.signal.aborted) setPipelines(all);
    })()
      .catch((cause) => {
        if (!controller.signal.aborted) setPipelineError(errorText(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setPipelineLoading(false);
      });
    return () => controller.abort();
  }, [workspace, readOnly, canPipeline, revision]);
  const isGit = (row: InsightRepository) => {
    const provider =
      row.provider ||
      data.connections.find((connection) => connection.id === row.connection_id)
        ?.provider;
    return provider
      ? provider === "git"
      : row.repository.includes("://") ||
          /^[^/@\s]+@[^/:\s]+:/.test(row.repository);
  };
  const incomplete =
    !selected.length ||
    selected.some(
      (row) =>
        !row.connection_id ||
        (!row.all_repositories && !row.repository.trim()) ||
        (enabled && isGit(row) && !row.pipeline_id),
    );
  useEffect(() => {
    onValidityChange(!readOnly && incomplete);
    return () => onValidityChange(false);
  }, [readOnly, incomplete, onValidityChange]);
  const add = () => {
    const remote = data.repositories.find((row) => row.url === choice.url);
    const repository =
      data.connection?.provider === "git"
        ? choice.url.trim()
        : (remote?.full_name ?? "");
    if (
      !data.connection ||
      !repository ||
      selected.some(
        (row) =>
          row.connection_id === choice.connection_id &&
          row.repository === repository,
      )
    )
      return;
    onChange([
      ...selected,
      { connection_id: choice.connection_id, repository, pipeline_id: "" },
    ]);
    setChoice((current) => ({ ...current, url: "" }));
  };
  return (
    <section
      aria-label={insightText("洞察仓库")}
      className="space-y-4 sm:col-span-2"
    >
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-medium">
          {insightText("选择仓库")}
          <span className="ml-1 text-destructive" aria-hidden="true">
            *
          </span>
        </h3>
        <span className="rounded-md bg-secondary px-2 py-1 text-xs tabular-nums text-secondary-foreground">
          {insightText("selectedRepositories", { count: selected.length })}
        </span>
      </div>
      <p className="text-xs text-muted-foreground">
        {insightText(
          "可选择多个连接中的仓库；同一仓库可加入不同洞察，各洞察的作者映射独立。",
        )}
      </p>
      {selected.length ? (
        <ul className="divide-y rounded-lg border">
          {selected.map((row, index) => (
            <li
              key={`${row.connection_id}/${row.repository}`}
              className="space-y-3 p-4"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex min-w-0 flex-1 items-center gap-3">
                  <GitBranch
                    aria-hidden="true"
                    className="size-4 shrink-0 text-muted-foreground"
                  />
                  <p className="min-w-0 break-all text-sm">
                    <span className="block text-xs text-muted-foreground">
                      {row.connection_name ||
                        data.connections.find(
                          (connection) => connection.id === row.connection_id,
                        )?.name ||
                        insightText("仓库连接")}
                    </span>
                    <span className="font-medium">
                      {row.all_repositories
                        ? insightText("全部可见仓库")
                        : row.repository}
                    </span>
                  </p>
                </div>
                {!readOnly && canSelect && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    aria-label={insightText("removeRepository", {
                      repository: row.repository,
                    })}
                    onClick={() =>
                      onChange(
                        selected.filter((_, position) => position !== index),
                      )
                    }
                  >
                    {insightText("移除")}
                  </Button>
                )}
              </div>
              {!readOnly && canPipeline ? (
                <AutoField
                  label={insightText("分析流水线")}
                  required={enabled && isGit(row)}
                  className="block space-y-2"
                >
                  <select
                    aria-label={insightText("pipelineFor", {
                      repository: row.repository,
                    })}
                    className={selectClass}
                    value={row.pipeline_id}
                    required={enabled && isGit(row)}
                    disabled={pipelineLoading || !!pipelineError}
                    onChange={(event) =>
                      onChange(
                        selected.map((item, position) =>
                          position === index
                            ? { ...item, pipeline_id: event.target.value }
                            : item,
                        ),
                      )
                    }
                  >
                    <option value="">
                      {isGit(row)
                        ? insightText("请选择分析流水线")
                        : insightText("使用平台接口")}
                    </option>
                    {row.pipeline_id &&
                      !pipelines.some(
                        (pipeline) => pipeline.id === row.pipeline_id,
                      ) && (
                        <option value={row.pipeline_id}>
                          {row.pipeline_name || insightText("已配置分析流水线")}
                        </option>
                      )}
                    {pipelines.map((pipeline) => (
                      <option key={pipeline.id} value={pipeline.id}>
                        {pipeline.name}
                      </option>
                    ))}
                  </select>
                </AutoField>
              ) : (
                <p className="text-xs text-muted-foreground">
                  {insightText("分析流水线：")}
                  {row.pipeline_name ||
                    (row.pipeline_id
                      ? insightText("已配置分析流水线")
                      : isGit(row)
                        ? insightText("未配置分析流水线")
                        : insightText("使用平台接口"))}
                </p>
              )}
              {isGit(row) && (
                <p className="text-xs text-muted-foreground">
                  {insightText(
                    "普通 Git 通过分析流水线采集；启用持续采集前须选择流水线。",
                  )}
                </p>
              )}
              {!readOnly && enabled && isGit(row) && !row.pipeline_id && (
                <p role="alert" className="text-sm text-destructive">
                  {insightText(
                    "请为此 Git 仓库选择分析流水线，或关闭持续采集后保存。",
                  )}
                </p>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <p
          role="status"
          className="rounded-lg border border-dashed p-5 text-sm text-muted-foreground"
        >
          {insightText("尚未选择仓库。")}
        </p>
      )}
      {!readOnly && canSelect && (
        <div className="grid min-w-0 gap-4 rounded-lg bg-muted/40 p-4 sm:grid-cols-2">
          <ConnectionRepositoryFields
            navigation={navigation}
            workspace={workspace}
            data={data}
            connectionId={choice.connection_id}
            repositoryUrl={choice.url}
            repositoryLabel={insightText("添加洞察仓库")}
            onChange={(selection) =>
              setChoice({
                connection_id: selection.connection_id,
                url: selection.url,
              })
            }
          />
          <div className="sm:col-span-2">
            <Button
              type="button"
              variant="outline"
              disabled={
                !choice.url.trim() ||
                !data.connection ||
                data.loading ||
                !!data.error
              }
              onClick={add}
            >
              <Plus aria-hidden="true" className="size-4" />
              {insightText("加入洞察")}
            </Button>
            <p className="mt-2 text-xs leading-5 text-muted-foreground">
              {insightText("选择仓库后，点击加入洞察完成添加。")}
            </p>
          </div>
        </div>
      )}
      {!readOnly && !canSelect && (
        <p className="text-sm text-muted-foreground">
          {insightText("更改仓库范围需要仓库读取权限。")}
        </p>
      )}
      {!readOnly && !canPipeline && (
        <p className="text-xs text-muted-foreground">
          {insightText("更换分析流水线需要流水线读取及执行权限。")}
        </p>
      )}
      {!readOnly && pipelineLoading && (
        <p role="status" className="text-sm">
          {insightText("正在读取分析流水线…")}
        </p>
      )}
      {!readOnly && pipelineError && (
        <div>
          <p role="alert" className="text-sm text-destructive">
            {pipelineError}
          </p>
          <Button
            type="button"
            variant="ghost"
            onClick={() => setRevision((current) => current + 1)}
          >
            {insightText("重试读取流水线")}
          </Button>
        </div>
      )}
    </section>
  );
}

/** Edit identities as rows while preserving existing contributor IDs and metadata. */
export function CodeInsightAliases({
  value,
  readOnly,
  onChange,
  onValidityChange,
}: Omit<Props, "workspace" | "permissions">) {
  useTranslation("insight-domain");
  const aliases = Array.isArray(value)
    ? (value as Record<string, unknown>[])
    : [];
  const incomplete = aliases.some(
    (alias) =>
      !String(alias.nickname ?? "").trim() ||
      (!String(alias.name ?? "").trim() && !String(alias.email ?? "").trim()),
  );
  useEffect(() => {
    onValidityChange(!readOnly && incomplete);
    return () => onValidityChange(false);
  }, [readOnly, incomplete, onValidityChange]);
  const update = (index: number, key: string, text: string) =>
    onChange(
      aliases.map((alias, position) =>
        position === index ? { ...alias, [key]: text } : alias,
      ),
    );
  return (
    <section
      aria-label={insightText("作者昵称映射")}
      className="space-y-4 sm:col-span-2"
    >
      <p className="text-sm leading-6 text-muted-foreground">
        {insightText(
          "将不同提交姓名或邮箱映射到同一昵称，统一贡献统计。仅影响当前洞察，不会重新采集代码。",
        )}
      </p>
      {aliases.length === 0 && (
        <p
          role="status"
          className="rounded-lg border border-dashed p-5 text-sm text-muted-foreground"
        >
          {insightText("尚无作者映射，统计将使用原始提交身份。")}
        </p>
      )}
      <div className="space-y-3">
        {aliases.map((alias, index) => (
          <div
            key={index}
            className="grid min-w-0 items-start gap-4 rounded-lg border p-4 sm:grid-cols-2 lg:grid-cols-[1fr_1fr_1fr_auto]"
          >
            {(
              [
                ["name", "提交姓名"],
                ["email", "提交邮箱"],
                ["nickname", "统一昵称"],
              ] as const
            ).map(([key, label]) => (
              <AutoField
                key={key}
                label={insightText(label)}
                required={key === "nickname"}
                className="block min-w-0 space-y-2"
              >
                {readOnly ? (
                  <p className="break-all text-sm">
                    {String(alias[key] || "—")}
                  </p>
                ) : (
                  <Input
                    aria-label={insightText(label) + " " + (index + 1)}
                    value={String(alias[key] ?? "")}
                    required={key === "nickname"}
                    onChange={(event) => update(index, key, event.target.value)}
                  />
                )}
              </AutoField>
            ))}
            {!readOnly && (
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="sm:mt-6"
                aria-label={insightText("removeAlias", { index: index + 1 })}
                onClick={() =>
                  onChange(aliases.filter((_, position) => position !== index))
                }
              >
                <Trash2 aria-hidden="true" className="size-4" />
              </Button>
            )}
          </div>
        ))}
      </div>
      {!readOnly && (
        <Button
          type="button"
          variant="outline"
          onClick={() =>
            onChange([...aliases, { name: "", email: "", nickname: "" }])
          }
        >
          <Plus aria-hidden="true" className="size-4" />
          {insightText("添加作者映射")}
        </Button>
      )}
      {!readOnly && incomplete && (
        <p role="alert" className="text-sm text-destructive">
          {insightText("每条作者映射须填写姓名或邮箱，以及显示昵称。")}
        </p>
      )}
    </section>
  );
}
