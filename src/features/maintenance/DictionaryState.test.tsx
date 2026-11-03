import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { listConfigs } from "../../app/resources";
import { commonText, i18n } from "../../shared/i18n/i18n";
import { DictionaryContext, DictionaryState, dictionaryStateLabel } from "./DictionaryState";

describe("workspace status presentation", () => {
  const dictionaries = {
    "pipeline-state": [{ value: "running", label: "Building", color: "#123456" }, { value: "queued", label: "In line" }],
    "deployment-state": [{ value: "running", label: "Deploying", color: "#ffffff" }],
    "cron-state": [{ value: "running", label: "Scheduled work" }],
    "alert-state": [{ value: "firing", label: "Attention" }],
    "repository-state": [{ value: "refreshing", label: "Renewing access" }],
    "insight-state": [{ value: "running", label: "Collecting" }],
  };

  it("keeps labels scoped by domain and computes readable colors", () => {
    const html = renderToStaticMarkup(<DictionaryContext.Provider value={dictionaries}>
      <DictionaryState dictionary="pipeline-state" value="running">Running</DictionaryState>
      <DictionaryState dictionary="deployment-state" value="running">Running</DictionaryState>
      <DictionaryState dictionary="insight-state" value="running">Running</DictionaryState>
    </DictionaryContext.Provider>);
    expect(html).toContain("Building");
    expect(html).toContain("Deploying");
    expect(html).toContain("Collecting");
    expect(html).toContain("background-color:#123456;color:#ffffff");
    expect(html).toContain("background-color:#ffffff;color:#000000");
    expect(dictionaryStateLabel(dictionaries, "deployment-state", "running", "Fallback")).toBe("Deploying");
  });

  it("preserves legacy and unknown-state presentation when preferences are absent", () => {
    const html = renderToStaticMarkup(<DictionaryState dictionary="pipeline-state" value="new-state"><strong>Unknown state</strong></DictionaryState>);
    expect(html).toBe("<strong>Unknown state</strong>");
  });

  it("localizes canonical repository fallbacks in all six languages and retains unknown values", async () => {
    const previous = i18n.language;
    try {
      for (const locale of ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"]) {
        await i18n.changeLanguage(locale);
        const column = listConfigs("ops")["code-connections"]!.columns.find(item => item.key === "state")!;
        for (const [state, key] of [["refreshing", "state.refreshingAuthorization"], ["reauthorize", "state.reauthorizationRequired"]]) {
          const html = renderToStaticMarkup(<>{column.render!({ state })}</>);
          expect(html, locale).toBe(commonText(key!));
          expect(html, locale).not.toContain(state!);
        }
        expect(renderToStaticMarkup(<>{column.render!({ state: "future-state" })}</>)).toBe("future-state");
      }
    } finally { await i18n.changeLanguage(previous); }
  });

  it("connects every operational list and preserves configured filter ordering", () => {
    const configs = listConfigs("ops", new Set(), undefined, undefined, dictionaries);
    for (const [resource, state, label] of [
      ["pipeline-runs", "running", "Building"],
      ["deployments", "running", "Deploying"],
      ["deployment-revisions", "running", "Deploying"],
      ["cron-runs", "running", "Scheduled work"],
      ["alert-events", "firing", "Attention"],
      ["code-connections", "refreshing", "Renewing access"],
    ]) {
      const column = configs[resource!]!.columns.find(item => item.key === "state")!;
      const html = renderToStaticMarkup(<DictionaryContext.Provider value={dictionaries}>{column.render!({ state })}</DictionaryContext.Provider>);
      expect(html, resource).toContain(label);
    }
    const options = configs["pipeline-runs"]!.filters!.find(filter => filter.key === "state")!.options;
    expect(Array.isArray(options) && options.map(option => option.value)).toEqual(["running", "queued"]);
  });
});
