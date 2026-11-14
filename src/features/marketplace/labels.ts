import { commonText } from "../../shared/i18n/i18n";

export const marketProviders = [
  { value: "github", label: "GitHub" },
  { value: "gitlab", label: "GitLab" },
  { value: "gitee", label: "Gitee" },
  { value: "codeup", label: "Codeup" },
  // Reads the index by a shallow Git fetch: no hosting API, so no API rate limit.
  { value: "git", label: "Git" },
];

/** Source kind label shared by the platform sources page and the workspace market list. */
export const trustLabel = (trust: string) =>
  ({ official: commonText("resource-domain:market.official"), community: commonText("resource-domain:market.community"), workspace: commonText("resource-domain:market.workspaceSource") })[trust] ?? trust;
