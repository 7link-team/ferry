import assert from "node:assert/strict";
import test from "node:test";
import {
  buildCatalog,
  currentDescription,
  selectors,
} from "./inventory-n8n.mjs";

test("default description wins over a newer non-default and an older legacy record", () => {
  const legacy = { name: "app", version: 1, defaultVersion: 2.7 };
  const preferred = { name: "app", version: [2, 2.7], defaultVersion: 2.7 };
  const preview = { name: "app", version: 3 };
  assert.equal(currentDescription([legacy, preview, preferred]), preferred);
  assert.equal(
    currentDescription([{ version: 1 }, { version: [2, 2.5] }]).version[1],
    2.5,
  );
});

test("same operation ID on different resources keeps both contexts; repeated declarations collapse", () => {
  const field = (resource) => ({
    name: "operation",
    type: "options",
    displayOptions: { show: { resource: [resource] } },
    options: [{ name: "Get", value: "get" }],
  });
  const entries = selectors({
    properties: [field("issue"), field("file"), field("issue")],
  });
  assert.equal(entries.length, 2);
  assert.deepEqual(
    entries.map((entry) => entry.conditions.show.resource[0]),
    ["issue", "file"],
  );
});

test("dynamic selectors are explicit and nested selectors preserve field paths and option conditions", () => {
  const entries = selectors({
    properties: [
      {
        name: "operation",
        type: "options",
        typeOptions: { loadOptionsMethod: "remote" },
      },
      {
        name: "settings",
        type: "fixedCollection",
        options: [
          {
            name: "trigger",
            values: [
              {
                name: "events",
                type: "multiOptions",
                options: [
                  {
                    name: "Created",
                    value: "created",
                    displayOptions: { show: { mode: ["live"] } },
                  },
                ],
              },
            ],
          },
        ],
      },
    ],
  });
  assert.equal(entries[0].dynamic, true);
  assert.equal(entries[1].field, "settings.trigger.events");
  assert.deepEqual(entries[1].optionConditions, { show: { mode: ["live"] } });
});

test("generic Custom API Call is marked separately from a service operation", () => {
  const entries = selectors({
    properties: [
      {
        name: "operation",
        type: "options",
        options: [
          { name: "Get", value: "get" },
          { name: "Custom API Call", value: "__CUSTOM_API_CALL__" },
        ],
      },
    ],
  });
  assert.equal(entries[0].genericCustomApi, false);
  assert.equal(entries[1].genericCustomApi, true);
});

test("send-and-wait webhooks do not turn actions or human review tools into triggers", () => {
  const action = {
    name: "n8n-nodes-base.app",
    version: 1,
    inventoryPackage: "n8n-nodes-base",
    group: ["output"],
    webhooks: [{ name: "resume" }],
  };
  const review = {
    ...action,
    name: "n8n-nodes-base.appHitlTool",
    inventoryPackage: undefined,
  };
  const catalog = buildCatalog([action], [action, review], [], {});
  assert.equal(catalog.summary.runtime.triggers, 0);
  assert.equal(catalog.summary.humanReviewToolVariants, 1);
  assert.equal(
    catalog.nodes.find((node) => node.variantKind === "human_review").parent,
    action.name,
  );
});

test("installed, disabled, generated Tool, and MCP nodes are counted separately without version inflation", () => {
  const installed = [
    {
      name: "n8n-nodes-base.app",
      displayName: "App",
      version: 1,
      defaultVersion: 2,
      inventoryPackage: "n8n-nodes-base",
    },
    {
      name: "n8n-nodes-base.app",
      displayName: "App",
      version: 2,
      defaultVersion: 2,
      inventoryPackage: "n8n-nodes-base",
      credentials: [{ name: "api" }],
      codex: { categories: ["Miscellaneous "] },
    },
    {
      name: "n8n-nodes-base.disabled",
      version: 1,
      inventoryPackage: "n8n-nodes-base",
    },
  ];
  const runtime = [
    ...installed.slice(0, 2),
    { name: "n8n-nodes-base.appTool", version: 2, outputs: ["ai_tool"] },
    {
      name: "@n8n/mcp-registry.remote",
      version: 1,
      outputs: [{ type: "ai_tool" }],
    },
  ];
  const catalog = buildCatalog(
    installed,
    runtime,
    [
      {
        name: "api",
        displayName: "API",
        properties: [{ name: "secret", default: "DO_NOT_EXPORT" }],
      },
    ],
    {},
  );
  assert.equal(catalog.summary.packaged.nodes, 2);
  assert.equal(catalog.summary.loadedPackaged.nodes, 1);
  assert.equal(catalog.summary.runtime.nodes, 3);
  assert.equal(catalog.summary.runtimeDescriptionRecords, 4);
  assert.equal(catalog.summary.toolVariants.nodes, 1);
  assert.equal(catalog.summary.mcpCatalog.nodes, 1);
  assert.deepEqual(catalog.summary.excludedPackagedTypes, [
    "n8n-nodes-base.disabled",
  ]);
  assert.equal(
    catalog.nodes.find((node) => node.type.endsWith("appTool")).parent,
    "n8n-nodes-base.app",
  );
  assert.deepEqual(
    catalog.nodes.find((node) => node.type.endsWith(".app")).categories,
    ["Miscellaneous"],
  );
  assert.deepEqual(catalog.credentials[0].nodes, ["n8n-nodes-base.app"]);
  assert.ok(!JSON.stringify(catalog).includes("DO_NOT_EXPORT"));
});
