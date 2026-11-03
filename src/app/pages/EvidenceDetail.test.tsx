import { renderToStaticMarkup } from "react-dom/server";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { ReportSummary, ReviewEvidence } from "./EvidenceDetail";

it("selects only filtered report revisions and hides stale or empty evidence", async () => {
  await i18n.changeLanguage("en-US");
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onChoose = vi.fn();
  const week = {
    id: "week",
    period: "week",
    definition_version: 1,
    state: "complete",
    snapshot: {
      commits: [{ message: "Weekly immutable evidence" }],
      summary: { commits: 12 },
    },
  };
  const day = {
    id: "day",
    period: "day",
    definition_version: 2,
    state: "complete",
    snapshot: {
      commits: [{ message: "Daily immutable evidence" }],
      summary: { commits: 3 },
    },
  };
  const render = async (
    items: (typeof week)[],
    runId: string,
    selected: typeof week,
  ) =>
    act(async () =>
      root.render(
        <ReportSummary
          items={items}
          runId={runId}
          selected={selected}
          workspace="2"
          endpoint="/workspaces/2/insight-reports/r"
          onChoose={onChoose}
        />,
      ),
    );
  try {
    await render([week], "week", week);
    expect(host.textContent).toContain("Weekly immutable evidence");
    await render([day], "week", week);
    expect(onChoose).toHaveBeenLastCalledWith("day");
    expect(host.textContent).not.toContain("Weekly immutable evidence");
    await render([day], "day", day);
    expect(host.textContent).toContain("Daily immutable evidence");
    expect(host.querySelector("a")?.href).toContain("/runs/day/export");
    await render([], "day", day);
    expect(onChoose).toHaveBeenLastCalledWith("");
    expect(host.textContent).not.toContain("Daily immutable evidence");
  } finally {
    await act(async () => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  }
});

it("distinguishes unavailable and absent evidence from passing tests", async () => {
  await i18n.changeLanguage("en-US");
  const unavailable = renderToStaticMarkup(
    <ReviewEvidence
      workspace="2"
      value={{ available: false, reason: "permission" }}
    />,
  );
  expect(unavailable).toContain("Code review read permission is required");
  expect(unavailable).not.toContain("Tests passed");
  const empty = renderToStaticMarkup(
    <ReviewEvidence
      workspace="2"
      value={{
        available: true,
        total: 0,
        tests_passed: 0,
        tests_failed: 0,
        findings: 0,
        runs: [],
      }}
    />,
  );
  expect(empty).toContain("No execution evidence");
  expect(empty).not.toContain("Tests passed");
});
it("links real immutable run SHAs and shows independent test and AI outcomes", async () => {
  await i18n.changeLanguage("en-US");
  const html = renderToStaticMarkup(
    <ReviewEvidence
      workspace="2"
      value={{
        available: true,
        total: 1,
        tests_passed: 0,
        tests_failed: 1,
        findings: 2,
        runs: [
          {
            id: "run",
            review_id: "review",
            repository: "team/api",
            head_sha: "immutable-sha",
            state: "completed",
            test_state: "failed",
            ai_state: "completed_findings",
            finding_count: 2,
            pipeline_run_id: "pipeline",
          },
        ],
        unavailable_repositories: [
          { repository: "team/private", reason: "credential" },
        ],
      }}
    />,
  );
  expect(html).toContain("immutable-sha");
  expect(html).toContain("Tests failed");
  expect(html).toContain("AI completed: findings");
  expect(html).toContain("code-reviews/review");
  expect(html).toContain("run=run");
  expect(html).toContain("pipeline-runs/pipeline");
  expect(html).toContain("Repository credential unavailable");
});
