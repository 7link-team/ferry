import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { listConfigs } from "../../app/resources";
import { commonText, i18n } from "../../shared/i18n/i18n";

it("ignores stale state dictionaries and keeps canonical filters and translated labels", async () => {
 const previous = i18n.language;
 try {
  for (const locale of ["zh-CN", "en-US", "zh-TW", "ja-JP", "ko-KR", "ms-MY"]) {
   await i18n.changeLanguage(locale);
   const preferences = { "pipeline-state": [{value:"running", label:"Injected label"}], "repository-state": [{value:"refreshing",label:"Injected label"}] };
   const configs = listConfigs("ops",new Set(),undefined,undefined,preferences);
   const column = configs["code-connections"]!.columns.find(item => item.key === "state")!;
   expect(renderToStaticMarkup(<>{column.render!({state:"refreshing"})}</>)).toBe(commonText("state.refreshingAuthorization"));
   expect(renderToStaticMarkup(<>{column.render!({state:"future-state"})}</>)).toBe("future-state");
   const states = configs["pipeline-runs"]!.filters!.find(filter => filter.key === "state")!.options;
   expect(Array.isArray(states) && states.length).toBeGreaterThan(1);
   expect(renderToStaticMarkup(<>{configs["pipeline-runs"]!.columns.find(item => item.key === "state")!.render!({state:"running"})}</>)).not.toContain("Injected label");
  }
 } finally { await i18n.changeLanguage(previous); }
});
