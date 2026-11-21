import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import * as apiModule from "../../shared/api/api";
import { commonText, workflowText as t } from "../../shared/i18n/i18n";
import { WorkflowAIParameters } from "./WorkflowAIParameters";
import { WorkflowAssetContext } from "./WorkflowAssets";

it("distinguishes unknown cost from known and legacy zero cost and identifies conversation calls", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const rows = [
    { id: "1", actual_micros: null, cost_known: false },
    { id: "2", actual_micros: 0, cost_known: false },
    { id: "3", actual_micros: 0, cost_known: true },
    { id: "4", actual_micros: 0 },
  ].map(row => ({ state: "succeeded", currency: "USD", reserved_micros: 0, execution_id: "0", conversation_id: "9", source_type: "conversation", ...row }));
  const mocked = vi.spyOn(apiModule, "api").mockImplementation(async path => ({ items: String(path).endsWith("/ai-calls") ? rows : [] }) as never);
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<WorkflowAssetContext.Provider value={{ workspace: "1", workflow: "2" }}>
      <WorkflowAIParameters type="ai.llm.chat" value={{}} readOnly={false} change={() => {}} />
    </WorkflowAssetContext.Provider>));
    const ledger = [...host.querySelectorAll("button")].find(button => button.textContent === t("aiLedger"))!;
    await act(async () => ledger.click());
    const costs = [...host.querySelectorAll("p")].map(node => node.textContent).filter(text => text?.startsWith(t("aiActualCost")));
    expect(costs).toEqual([
      `${t("aiActualCost")}: ${t("aiCostUnknown")}`,
      `${t("aiActualCost")}: ${t("aiCostUnknown")}`,
      `${t("aiActualCost")}: 0`,
      `${t("aiActualCost")}: 0`,
    ]);
    expect(host.textContent).toContain(`${commonText("conversations:title")} 9`);
  } finally { await act(async () => root.unmount()); host.remove(); mocked.mockRestore(); }
});
