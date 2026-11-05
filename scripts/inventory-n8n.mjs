/** Extract factual catalog metadata, never node implementations or credentials' values.
 * Inputs are an installed n8n and authenticated /types/{nodes,credentials}.json snapshots.
 */
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const packages = ["n8n-nodes-base", "@n8n/n8n-nodes-langchain"];
const array = (value) =>
  value === undefined ? [] : Array.isArray(value) ? value : [value];
const versions = (node) =>
  array(node.version).filter((value) => typeof value === "number");
const unique = (values) => [...new Set(values)].sort();
const byName = (a, b) => a.name.localeCompare(b.name, "en");
const canonical = (value) =>
  JSON.stringify(value, (_key, child) =>
    child && typeof child === "object" && !Array.isArray(child)
      ? Object.fromEntries(
          Object.entries(child).sort(([a], [b]) => a.localeCompare(b)),
        )
      : child,
  );

export function currentDescription(descriptions) {
  const preferred = descriptions.find(
    (node) => node.defaultVersion !== undefined,
  )?.defaultVersion;
  const candidates =
    preferred === undefined
      ? descriptions
      : descriptions.filter((node) => versions(node).includes(preferred));
  return [...(candidates.length ? candidates : descriptions)].sort(
    (a, b) => Math.max(...versions(b), 0) - Math.max(...versions(a), 0),
  )[0];
}

/** Keep resource/visibility predicates: equal operation IDs in different resources are different entries. */
export function selectors(node) {
  const result = [];
  const visit = (properties, prefix = "") => {
    for (const property of properties ?? []) {
      const field = `${prefix}${property.name ?? property.displayName ?? "?"}`;
      const kind =
        property.name === "resource"
          ? "resource"
          : property.name === "operation"
            ? "operation"
            : /^(events?|triggerOn|updates)$/i.test(property.name ?? "")
              ? "event"
              : undefined;
      if (kind) {
        const values = (property.options ?? []).filter(
          (option) =>
            Object.hasOwn(option, "value") && typeof option.value !== "object",
        );
        for (const option of values)
          result.push({
            kind,
            field,
            value: option.value,
            genericCustomApi: option.value === "__CUSTOM_API_CALL__",
            name: option.name ?? String(option.value),
            conditions: property.displayOptions ?? {},
            optionConditions: option.displayOptions ?? {},
          });
        if (!values.length)
          result.push({
            kind,
            field,
            dynamic: true,
            conditions: property.displayOptions ?? {},
            optionConditions: {},
          });
      }
      // Containers may hold nested parameter groups; never traverse arbitrary metadata/code.
      if (["collection", "fixedCollection"].includes(property.type)) {
        for (const option of property.options ?? []) {
          if (option.values) visit(option.values, `${field}.${option.name}.`);
          else if (option.name) visit([option], `${field}.`);
        }
      }
    }
  };
  visit(node.properties);
  return [...new Map(result.map((item) => [canonical(item), item])).values()];
}

function groupByName(nodes) {
  const groups = new Map();
  for (const node of nodes) {
    if (!node.name)
      throw new Error("Node description is missing its type identifier");
    groups.set(node.name, [...(groups.get(node.name) ?? []), node]);
  }
  return groups;
}

export function buildCatalog(
  installed,
  runtimeNodes,
  runtimeCredentials,
  metadata,
) {
  const loaded = groupByName(runtimeNodes);
  const all = groupByName(installed);
  for (const [name, descriptions] of loaded)
    if (!all.has(name)) all.set(name, descriptions);
  const nodes = [...all]
    .map(([type, installedDescriptions]) => {
      const runtimeDescriptions = loaded.get(type);
      const descriptions = runtimeDescriptions ?? installedDescriptions;
      const selected = currentDescription(descriptions);
      const installedNode = installedDescriptions.find(
        (node) => node.inventoryPackage,
      );
      const source = installedNode
        ? "packaged"
        : type.startsWith("@n8n/mcp-registry.")
          ? "mcp_catalog"
          : "tool_variant";
      const variantKind =
        source === "tool_variant"
          ? type.endsWith("HitlTool")
            ? "human_review"
            : "tool"
          : null;
      const parentCandidate = type.replace(
        variantKind === "human_review" ? /HitlTool$/ : /Tool$/,
        "",
      );
      const parent =
        source === "tool_variant" && all.has(parentCandidate)
          ? parentCandidate
          : null;
      const outputs = array(selected.outputs).flatMap((output) =>
        typeof output === "string"
          ? [output]
          : output?.type
            ? [output.type]
            : [],
      );
      const selection = selectors(selected);
      return {
        type,
        name: selected.displayName ?? type,
        source,
        package:
          installedNode?.inventoryPackage ??
          type.slice(0, type.lastIndexOf(".")),
        parent,
        variantKind,
        loaded: !!runtimeDescriptions,
        hidden: selected.hidden === true,
        deprecated: selected.deprecated === true,
        trigger:
          array(selected.group).includes("trigger") || !!selected.polling,
        hasWebhook: !!selected.webhooks?.length,
        outputs,
        groups: selected.group ?? [],
        categories: unique(
          (selected.codex?.categories ?? []).map((category) => category.trim()),
        ),
        versions: [...new Set(descriptions.flatMap(versions))].sort(
          (a, b) => a - b,
        ),
        selectedVersions: versions(selected),
        defaultVersion:
          selected.defaultVersion ?? Math.max(...versions(selected), 0),
        descriptionRecords: descriptions.length,
        credentials: unique(
          (selected.credentials ?? []).map((credential) => credential.name),
        ),
        resources: selection.filter((item) => item.kind === "resource"),
        operations: selection.filter((item) => item.kind === "operation"),
        events: selection.filter((item) => item.kind === "event"),
        parameterFields: (selected.properties ?? []).map((property) => ({
          name: property.name,
          type: property.type,
        })),
      };
    })
    .sort(byName);
  const credentials = [...groupByName(runtimeCredentials)]
    .map(([type, entries]) => ({
      type,
      name: entries[0].displayName ?? type,
      extends: entries[0].extends ?? [],
      nodes: nodes
        .filter((node) => node.loaded && node.credentials.includes(type))
        .map((node) => node.type),
    }))
    .sort(byName);
  const packaged = nodes.filter((node) => node.source === "packaged");
  const live = nodes.filter((node) => node.loaded);
  const count = (list) => ({
    nodes: list.length,
    triggers: list.filter((node) => node.trigger).length,
    hidden: list.filter((node) => node.hidden).length,
    deprecated: list.filter((node) => node.deprecated).length,
    staticOperationEntries: list.reduce(
      (total, node) =>
        total +
        node.operations.filter((operation) => !operation.dynamic).length,
      0,
    ),
    genericCustomApiEntries: list.reduce(
      (total, node) =>
        total +
        node.operations.filter((operation) => operation.genericCustomApi)
          .length,
      0,
    ),
    dedicatedOperationEntries: list.reduce(
      (total, node) =>
        total +
        node.operations.filter(
          (operation) => !operation.dynamic && !operation.genericCustomApi,
        ).length,
      0,
    ),
  });
  return {
    metadata,
    summary: {
      packaged: count(packaged),
      loadedPackaged: count(packaged.filter((node) => node.loaded)),
      runtime: count(live),
      toolVariants: count(
        live.filter((node) => node.source === "tool_variant"),
      ),
      humanReviewToolVariants: live.filter(
        (node) => node.variantKind === "human_review",
      ).length,
      mcpCatalog: count(live.filter((node) => node.source === "mcp_catalog")),
      runtimeDescriptionRecords: runtimeNodes.length,
      credentialTypes: credentials.length,
      packages: Object.fromEntries(
        packages.map((pkg) => [
          pkg,
          count(packaged.filter((node) => node.package === pkg)),
        ]),
      ),
      excludedPackagedTypes: packaged
        .filter((node) => !node.loaded)
        .map((node) => node.type),
      categories: Object.fromEntries(
        unique(packaged.flatMap((node) => node.categories)).map((category) => [
          category,
          packaged.filter((node) => node.categories.includes(category)).length,
        ]),
      ),
    },
    nodes,
    credentials,
  };
}

const cell = (value) =>
  String(value ?? "")
    .replaceAll("|", "\\|")
    .replaceAll(/\r?\n/g, " ");
const csv = (rows) =>
  "\uFEFF" +
  rows
    .map((row) =>
      row
        .map((value) => `"${String(value ?? "").replaceAll('"', '""')}"`)
        .join(","),
    )
    .join("\r\n") +
  "\r\n";
const compactSelector = (selector) =>
  `${selector.name ?? "动态选项"}${selector.value === undefined ? "" : ` (${selector.value})`}`;

function markdown(catalog) {
  const { summary: s, metadata: m, nodes } = catalog;
  const lines = [
    `# n8n ${m.n8nVersion} 内置节点实机清单`,
    "",
    `提取时间：${m.extractedAt}。来自本机实际安装包及已登录运行实例的节点目录，不采用官网营销数量。`,
    "",
    "## 数量与口径",
    "",
    "| 项目 | 数量 |",
    "|---|---:|",
    `| 安装包内独立节点类型 | ${s.packaged.nodes} |`,
    ...Object.entries(s.packages).map(
      ([pkg, counts]) => `| ${pkg}（${m.packages[pkg]}） | ${counts.nodes} |`,
    ),
    `| 其中运行实例加载的包内类型 | ${s.loadedPackaged.nodes} |`,
    `| 运行实例另外生成的 Tool 变体 | ${s.toolVariants.nodes} |`,
    `| 其中人工审核 Tool 变体（已包含在上一行） | ${s.humanReviewToolVariants} |`,
    `| 运行实例提供的 MCP 目录类型 | ${s.mcpCatalog.nodes} |`,
    `| 运行目录独立类型合计 | ${s.runtime.nodes} |`,
    `| 运行目录包含历史版本的描述记录（不当成类型数） | ${s.runtimeDescriptionRecords} |`,
    `| 运行目录凭据类型 | ${s.credentialTypes} |`,
    `| 包内节点静态 operation 选项记录（见下方限制） | ${s.packaged.staticOperationEntries} |`,
    `| 其中专用操作选项记录 | ${s.packaged.dedicatedOperationEntries} |`,
    `| 其中通用 Custom API Call 入口记录 | ${s.packaged.genericCustomApiEntries} |`,
    "",
    "- 一个节点类型不等于一个应用：同一应用可有普通节点、触发节点、模型节点；Tool 变体单列、不计作新增应用。其中人工审核变体只适用发送并等待等审核参数，继承的完整参数描述不代表所有操作都能作为审核工具执行。",
    "- MCP 目录表示连接远端服务的入口；远端工具列表取决于服务与授权，不能按目录项声称内置了全部远端实现。",
    "- 按类型标识去重，资源与操作取默认版本对应的参数描述；保留条件声明，不把旧版与新版操作简单相加。",
    "- 静态操作数是声明的选项记录数，不是已测试能力数；保留 resource、版本及其它显示条件，相同操作名可能属于不同资源。动态加载操作、远端工具、无 operation 选择器的节点不在该数内。",
    "- 运行实例会注入 Custom API Call 通用选项；在数据中标记 genericCustomApi，并与专用操作分开统计，不将这些通用入口视为额外的原生服务操作。",
    "- 隐藏项、弃用项、默认排除项保留并标记；目录存在不等于普通新增节点搜索可见。触发标记依据 group=trigger 或 polling；带 webhook 的普通发送/等待节点不因此算作触发器。类别可重叠。",
    "- 本清单不包括另外安装的社区节点，不表示 Ferry 已兼容或实现这些能力；没有执行各服务的商业账号验收。",
    "- 只输出名称、标识、版本、分类及参数选项等目录事实，不复制节点实现、认证实现、图标、原始参数定义或任何凭据值。",
    "",
    "## 文件",
    "",
    "- [nodes.csv](nodes.csv)：完整节点表，可用表格筛选应用、类型、分类、隐藏/加载状态及操作数量。",
    "- [operations.csv](operations.csv)：静态操作及事件选项，含资源/显示条件；Tool 变体单独标识。",
    "- [credentials.csv](credentials.csv)：凭据类型及被哪些节点引用，不含密钥。",
    "- [catalog.json](catalog.json)：机器可读的同一份提取结果，含资源、操作、事件、版本与条件。",
    "",
    "## 分类统计（包内类型，可重复归类）",
    "",
    "| 分类 | 类型数 |",
    "|---|---:|",
    ...Object.entries(s.categories).map(
      ([category, count]) => `| ${cell(category)} | ${count} |`,
    ),
    "",
    "## 当前未加载的包内类型",
    "",
    ...s.excludedPackagedTypes.map((type) => `- ${type}`),
    "",
    "## 用户提到的示例（默认参数描述）",
    "",
  ];
  for (const type of [
    "n8n-nodes-base.git",
    "n8n-nodes-base.github",
    "n8n-nodes-base.gitlab",
    "n8n-nodes-base.slack",
    "@n8n/n8n-nodes-langchain.anthropic",
    "@n8n/n8n-nodes-langchain.lmChatAnthropic",
  ]) {
    const node = nodes.find((node) => node.type === type);
    if (!node) continue;
    lines.push(
      `### ${node.name}`,
      "",
      `类型：\`${node.type}\`；默认版本：${node.defaultVersion}；凭据：${node.credentials.join(", ") || "无"}。`,
      "",
    );
    if (node.resources.length)
      lines.push(
        `资源：${node.resources.map(compactSelector).join("、")}。`,
        "",
      );
    const operations = node.operations.filter(
      (operation) => !operation.dynamic,
    );
    if (operations.length)
      lines.push(
        "| 资源/显示条件 | 操作 | 标识 |",
        "|---|---|---|",
        ...operations.map(
          (operation) =>
            `| ${cell(JSON.stringify(operation.conditions))} | ${cell(operation.name)} | ${cell(operation.value)} |`,
        ),
        "",
      );
    else
      lines.push(
        "没有静态 operation 选择器；能力通过节点本身与参数配置提供，不能因此计为零能力。",
        "",
      );
  }
  for (const [title, filtered] of [
    [
      "基础包完整节点表",
      nodes.filter(
        (node) => node.source === "packaged" && node.package === packages[0],
      ),
    ],
    [
      "AI 包完整节点表",
      nodes.filter(
        (node) => node.source === "packaged" && node.package === packages[1],
      ),
    ],
    [
      "运行时 Tool 变体",
      nodes.filter((node) => node.source === "tool_variant"),
    ],
    ["运行时 MCP 目录", nodes.filter((node) => node.source === "mcp_catalog")],
  ]) {
    lines.push(
      `## ${title}（${filtered.length}）`,
      "",
      "| 名称 | 类型标识 | 版本 | 分类 | 静态操作数 | 状态 |",
      "|---|---|---|---|---:|---|",
    );
    for (const node of filtered)
      lines.push(
        `| ${cell(node.name)} | ${cell(node.type)} | ${node.defaultVersion} | ${cell(node.categories.join(", "))} | ${node.operations.filter((operation) => !operation.dynamic).length} | ${[node.loaded ? "已加载" : "未加载", node.trigger && "触发", node.hidden && "隐藏", node.deprecated && "弃用"].filter(Boolean).join(" / ")} |`,
      );
    lines.push("");
  }
  lines.push(
    "## 复现",
    "",
    "使用 scripts/inventory-n8n.mjs，指定安装的 node_modules 及已登录实例导出的 /types/nodes.json、/types/credentials.json。脚本只读输入；原始快照留在仓库外，不写入文档目录。",
    "",
    "此次只完成目录提取。下一阶段将这些目录事实与 Ferry 原有能力清单逐项映射；该进度只在 MEMORY.md 维护，不在此另建进度表。",
    "",
  );
  return lines.join("\n");
}

async function main() {
  const flags = Object.fromEntries(
    Array.from({ length: (process.argv.length - 2) / 2 }, (_, index) =>
      process.argv.slice(2 + index * 2, 4 + index * 2),
    ),
  );
  for (const flag of [
    "--modules",
    "--runtime-nodes",
    "--runtime-credentials",
    "--output",
  ])
    if (!flags[flag]) throw new Error(`Missing ${flag}`);
  const read = async (filename) =>
    JSON.parse(await fs.readFile(filename, "utf8"));
  const root = flags["--modules"];
  const installed = [];
  const packageVersions = {};
  for (const pkg of packages) {
    const dir = path.join(root, pkg);
    packageVersions[pkg] = (await read(path.join(dir, "package.json"))).version;
    const nodes = await read(path.join(dir, "dist/types/nodes.json"));
    const known = await read(path.join(dir, "dist/known/nodes.json"));
    if (
      new Set(nodes.map((node) => node.name)).size !==
        Object.keys(known).length ||
      nodes.some((node) => !known[node.name])
    )
      throw new Error(`${pkg}: known/types indexes disagree`);
    installed.push(
      ...nodes.map((node) => ({
        ...node,
        name: `${pkg}.${node.name}`,
        inventoryPackage: pkg,
      })),
    );
  }
  const n8nVersion = (await read(path.join(root, "n8n/package.json"))).version;
  const catalog = buildCatalog(
    installed,
    await read(flags["--runtime-nodes"]),
    await read(flags["--runtime-credentials"]),
    {
      n8nVersion,
      packages: packageVersions,
      extractedAt: new Date().toISOString(),
      method:
        "installed dist/types + dist/known and authenticated runtime /types endpoints; metadata only",
    },
  );
  const out = flags["--output"];
  await fs.mkdir(out, { recursive: true });
  await fs.writeFile(
    path.join(out, "catalog.json"),
    JSON.stringify(catalog, null, 2) + "\n",
  );
  await fs.writeFile(path.join(out, "README.md"), markdown(catalog));
  await fs.writeFile(
    path.join(out, "nodes.csv"),
    csv([
      [
        "type",
        "name",
        "source",
        "package",
        "parent",
        "variant_kind",
        "loaded",
        "hidden",
        "deprecated",
        "trigger",
        "default_version",
        "versions",
        "categories",
        "credential_types",
        "resources",
        "static_operation_entries",
        "static_event_entries",
      ],
      ...catalog.nodes.map((node) => [
        node.type,
        node.name,
        node.source,
        node.package,
        node.parent,
        node.variantKind,
        node.loaded,
        node.hidden,
        node.deprecated,
        node.trigger,
        node.defaultVersion,
        node.versions.join(";"),
        node.categories.join(";"),
        node.credentials.join(";"),
        node.resources.map(compactSelector).join(";"),
        node.operations.filter((op) => !op.dynamic).length,
        node.events.filter((op) => !op.dynamic).length,
      ]),
    ]),
  );
  await fs.writeFile(
    path.join(out, "operations.csv"),
    csv([
      [
        "node_type",
        "node_name",
        "source",
        "loaded",
        "selected_versions",
        "kind",
        "field",
        "option_name",
        "option_value",
        "dynamic",
        "generic_custom_api",
        "field_conditions",
        "option_conditions",
      ],
      ...catalog.nodes.flatMap((node) =>
        [...node.operations, ...node.events].map((op) => [
          node.type,
          node.name,
          node.source,
          node.loaded,
          node.selectedVersions.join(";"),
          op.kind,
          op.field,
          op.name,
          op.value,
          !!op.dynamic,
          !!op.genericCustomApi,
          JSON.stringify(op.conditions),
          JSON.stringify(op.optionConditions),
        ]),
      ),
    ]),
  );
  await fs.writeFile(
    path.join(out, "credentials.csv"),
    csv([
      ["type", "name", "extends", "referencing_node_types"],
      ...catalog.credentials.map((credential) => [
        credential.type,
        credential.name,
        credential.extends.join(";"),
        credential.nodes.join(";"),
      ]),
    ]),
  );
  console.log(JSON.stringify(catalog.summary, null, 2));
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
)
  await main();
