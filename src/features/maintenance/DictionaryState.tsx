import { createContext, useContext, type ReactNode } from "react";
import type { Option } from "../../shared/resources/types";
import { Badge } from "../../shared/ui/badge";
import { TagBadge } from "../tags/TagBadge";

export type PresentationDictionaries = Record<string, Option[]>;
export const DictionaryContext = createContext<PresentationDictionaries>({});

export const stateDictionary: Record<string, string> = {
  hosts: "host-state",
  "pipeline-runs": "pipeline-state",
  "pipeline-jobs": "pipeline-state",
  "pipeline-steps": "pipeline-state",
  deployments: "deployment-state",
  "deployment-revisions": "deployment-state",
  "cron-runs": "cron-state",
  "alert-events": "alert-state",
  "code-connections": "repository-state",
};

export function dictionaryStateLabel(dictionaries: PresentationDictionaries, dictionary: string, value: string, fallback: string) {
  return dictionaries[dictionary]?.find(option => option.value === value)?.label ?? fallback;
}

/** Preferences are loaded once by the shell, never by a row or status badge. */
export function DictionaryState({ dictionary, value, children }: { dictionary: string; value: string; children: ReactNode }) {
  const dictionaries = useContext(DictionaryContext);
  const option = dictionaries[dictionary]?.find(item => item.value === value);
  if (!option) return <>{children}</>;
  return option.color ? <TagBadge color={option.color}>{option.label}</TagBadge> : <Badge variant="outline">{option.label}</Badge>;
}
