import type { ResourceConfigContext } from "../../shared/resources/config";
import { type AnyRow, form, list } from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";
import { typedResourceForm, typedResourceList } from "./typed-resources";

export function maintenanceLists({
  workspace,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    settings: list(typedResourceList(workspace, "settings")),
    dictionaries: list(typedResourceList(workspace, "dictionaries")),
  };
}

export function maintenanceForms({}: ResourceConfigContext): Record<
  string,
  FormConfig<AnyRow>
> {
  return {
    settings: form(typedResourceForm("settings")),
    dictionaries: form(typedResourceForm("dictionaries")),
  };
}
