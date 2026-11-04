import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import messages from "../../shared/i18n/locales/reports-reviews-domain";
import { reportForm, reportActions } from "./config";
import { statusText } from "./messages";
import { reportTrend } from "./trend";
import { ReportScope, type Row, type ReportOptions } from "./ReportScope";

let host: HTMLDivElement;
let root: Root;
let changes: Row;
it("shows contributors and line evidence at the snapshot timezone boundary", () => {
  const data = reportTrend({
    timezone: "Asia/Kathmandu",
    trend: [
      {
        date: "2025-01-02",
        commits: 2,
        additions: 30,
        deletions: 5,
        effectives: 25,
      },
    ],
    commits: [
      {
        author_key: "canonical",
        nickname: "Combined",
        author_date: Date.parse("2025-01-01T18:30:00Z"),
        effectives: 10,
      },
      {
        author_key: "canonical",
        nickname: "Combined",
        author_date: Date.parse("2025-01-01T19:30:00Z"),
        effectives: 15,
      },
    ],
  });
  expect(data[0]).toMatchObject({
    commits: 2,
    additions: 30,
    deletions: 5,
    effectives: 25,
    authors: [
      {
        authorKey: "canonical",
        nickname: "Combined",
        commits: 2,
        effectives: 25,
      },
    ],
  });
});
const repositories = [
  { connection_id: "11", repository: "team/api" },
  { connection_id: "11", repository: "team/web" },
];
const choices: ReportOptions = {
  people: [
    {
      key: "alice",
      nickname: "Alice Engineering",
      name: "Alice Git",
      email: "alice@example.invalid",
      aliases: ["alice-old"],
    },
    { key: "bob", nickname: "Bob" },
  ],
  applications: [
    { id: "app", name: "Customer portal", repositories: [repositories[0]!] },
  ],
  repositories,
};
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  await i18n.changeLanguage("en-US");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => ({ ok: true, status: 200, json: async () => choices })),
  );
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});
function Harness() {
  const [draft, setDraft] = useState<Row>({
    insight_id: "31",
    dimension: "person",
    author_keys: [],
    applications: [],
  });
  changes = draft;
  return (
    <ReportScope
      workspace="2"
      draft={draft}
      readOnly={false}
      onPatch={(patch) => setDraft((current) => ({ ...current, ...patch }))}
      onValidityChange={() => {}}
    />
  );
}
it("shows each contributor name and email once while retaining distinct aliases", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => ({
        ...choices,
        people: [
          {
            key: "canonical",
            nickname: "Combined",
            name: "Combined",
            email: "person@example.invalid",
            aliases: [
              "Combined",
              "person@example.invalid",
              { name: "Old name", email: "person@example.invalid" },
            ],
          },
        ],
      }),
    })),
  );
  await act(async () => root.render(<Harness />));
  const label = [...host.querySelectorAll("label")].find((item) =>
    item.textContent?.includes("Combined"),
  )!;
  expect(label.textContent?.match(/Combined/g)).toHaveLength(1);
  expect(label.textContent?.match(/person@example.invalid/g)).toHaveLength(1);
  expect(label.textContent).toContain("Old name");
  await act(async () =>
    label.querySelector<HTMLButtonElement>("button")!.click(),
  );
  expect(changes.author_keys).toEqual(["canonical"]);
});
it("keeps canonical person keys and maps an application to multiple authorized repositories", async () => {
  await act(async () => root.render(<Harness />));
  const alice = [...host.querySelectorAll("label")].find((label) =>
    label.textContent?.includes("Alice Engineering"),
  )!;
  expect(alice.textContent).toContain("alice-old");
  await act(async () =>
    alice.querySelector<HTMLButtonElement>("button")!.click(),
  );
  expect(changes.author_keys).toEqual(["alice"]);
  await act(async () =>
    host.querySelector<HTMLInputElement>('input[value="application"]')!.click(),
  );
  expect(host.querySelector('input[value="person"]')).toHaveProperty(
    "checked",
    false,
  );
  const app = [...host.querySelectorAll("label")].find((label) =>
    label.textContent?.includes("Customer portal"),
  )!;
  await act(async () =>
    app.querySelector<HTMLButtonElement>("button")!.click(),
  );
  const web = [...host.querySelectorAll("label")].find((label) =>
    label.textContent?.includes("team/web"),
  )!;
  await act(async () =>
    web.querySelector<HTMLButtonElement>("button")!.click(),
  );
  expect(changes.applications).toEqual([
    { application_id: "app", repositories },
  ]);
  expect(host.textContent).not.toContain("Alice Engineering");
});
it("blocks saving a new scope when options cannot be authorized", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => ({
      ok: false,
      status: 403,
      json: async () => ({ error: "Repository access revoked" }),
    })),
  );
  const validity = vi.fn();
  await act(async () =>
    root.render(
      <ReportScope
        workspace="2"
        draft={{
          insight_id: "31",
          dimension: "person",
          author_keys: ["alice"],
        }}
        readOnly={false}
        onPatch={() => {}}
        onValidityChange={validity}
      />,
    ),
  );
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(
    "Repository access revoked",
  );
  expect(validity).toHaveBeenLastCalledWith(true);
});
it("uses flat generate payloads and preserves historical dates", async () => {
  const generate = reportActions("2")[0]!;
  await generate.action!(
    { id: "r" },
    { period: "quarter", period_start: "2026-07-01" },
  );
  expect(fetch).toHaveBeenLastCalledWith(
    "/api/workspaces/2/insight-reports/r/generate",
    expect.objectContaining({
      method: "POST",
      body: '{"period":"quarter","period_start":"2026-07-01"}',
    }),
  );
});
it("offers five report periods through the shared resource contract", () => {
  const report = reportForm("2");
  const periods = report.sections
    .flatMap((section) => section.fields)
    .find((field) => field.key === "periods")!;
  expect(periods.type).toBe("multiselect");
  expect(periods.options).toHaveLength(5);
});
it("translates every new message in six locales and separates task, test and AI outcomes", async () => {
  const keys = Object.keys(messages["en-US"]!);
  expect(Object.keys(messages)).toHaveLength(6);
  for (const [locale, catalog] of Object.entries(messages)) {
    expect(Object.keys(catalog), locale).toEqual(keys);
    for (const value of Object.values(catalog))
      expect(value, locale).not.toMatch(/\?\?\?|^$/);
    await i18n.changeLanguage(locale);
    expect(statusText("failed", "test")).not.toBe(statusText("failed"));
    expect(statusText("completed_findings", "ai")).not.toBe(
      statusText("completed_clear", "ai"),
    );
    expect(statusText("pending")).not.toBe("pending");
  }
});
