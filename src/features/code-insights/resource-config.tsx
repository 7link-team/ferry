import type { ResourceConfigContext } from "../../shared/resources/config";
import { type AnyRow, form, list } from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";
import { codeInsightForm, codeInsightList } from "../code-insights/config";

export function codeInsightsLists({
  workspace,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    "code-insights": list(codeInsightList(workspace)),
  };
}

export function codeInsightsForms({
  workspace,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    "code-insights": form(codeInsightForm(workspace)),
  };
}
