import { useEffect, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { base } from "../../shared/api/resource";
import type { Navigation } from "../../shared/routing/types";
import { Input } from "../../shared/ui/input";
import { Checkbox } from "../../shared/ui/checkbox";
import { text } from "./messages";

export type Repository = { connection_id: string; repository: string };
export type Application = {
  application_id: string;
  repositories: Repository[];
};
export type ReportOptions = {
  people: {
    key: string;
    nickname?: string;
    name?: string;
    email?: string;
    aliases?: unknown[];
  }[];
  applications: { id: string; name: string; repositories?: Repository[] }[];
  repositories: Repository[];
};
export type Row = Record<string, unknown>;
export function toggleValue<T>(values: T[], value: T, checked: boolean): T[] {
  return checked
    ? [...new Set([...values, value])]
    : values.filter((item) => item !== value);
}
export function ReportScope({
  navigation,
  workspace,
  draft,
  readOnly,
  onPatch,
  onValidityChange,
}: {
  navigation?: Navigation;
  workspace: string;
  draft: Partial<Row>;
  readOnly: boolean;
  onPatch: (patch: Partial<Row>) => void;
  onValidityChange: (blocked: boolean) => void;
}) {
  const [options, setOptions] = useState<ReportOptions>();
  const [error, setError] = useState("");
  const [search, setSearch] = useState("");
  const insight = String(draft.insight_id ?? "");
  useEffect(() => {
    let live = true;
    setOptions(undefined);
    setError("");
    if (insight)
      void api<ReportOptions>(
        `${base(workspace, "insight-reports")}/options?insight_id=${encodeURIComponent(insight)}`,
      )
        .then((data) => {
          if (live) setOptions(data);
        })
        .catch((cause) => {
          if (live) setError(errorText(cause));
        });
    return () => {
      live = false;
    };
  }, [workspace, insight]);
  const authors = Array.isArray(draft.author_keys)
    ? draft.author_keys.map(String)
    : [];
  const apps = (
    Array.isArray(draft.applications) ? draft.applications : []
  ) as Application[];
  const invalid =
    !insight ||
    !options ||
    !!error ||
    (draft.dimension === "application"
      ? !apps.length || apps.some((app) => !app.repositories.length)
      : !authors.length);
  useEffect(() => {
    onValidityChange(!readOnly && invalid);
    return () => onValidityChange(false);
  }, [invalid, readOnly, onValidityChange]);
  if (error)
    return (
      <p role="alert" className="text-destructive">
        {error}
      </p>
    );
  if (!insight) return <p>{text("source")}</p>;
  if (!options) return <p role="status">{text("loading")}</p>;
  const repoKey = (r: Repository) => `${r.connection_id}:${r.repository}`;
  return (
    <div className="min-w-0 space-y-4">
      <fieldset className="flex flex-wrap gap-4">
        <legend className="mb-2 font-medium">{text("dimension")}</legend>
        {["person", "application"].map((value) => (
          <label key={value} className="flex min-h-11 items-center gap-2">
            <input
              type="radio"
              name="report-dimension"
              value={value}
              checked={draft.dimension === value}
              disabled={readOnly}
              onChange={() => onPatch({ dimension: value })}
            />
            {text(value)}
          </label>
        ))}
      </fieldset>
      {draft.dimension !== "application" ? (
        <fieldset className="space-y-3">
          <legend className="font-medium">{text("people")}</legend>
          <Input
            aria-label={text("search")}
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
          <div className="flex flex-wrap gap-x-6 gap-y-2">
            {options.people
              .filter((person) =>
                JSON.stringify(person)
                  .toLowerCase()
                  .includes(search.toLowerCase()),
              )
              .map((person) => (
                <label
                  key={person.key}
                  className="flex min-h-11 items-center gap-2"
                >
                  <Checkbox
                    checked={authors.includes(person.key)}
                    disabled={readOnly}
                    onCheckedChange={(checked) =>
                      onPatch({
                        author_keys: toggleValue(
                          authors,
                          person.key,
                          !!checked,
                        ),
                      })
                    }
                  />
                  <span>
                    {person.nickname ||
                      person.name ||
                      person.email ||
                      person.key}
                    <small className="block text-muted-foreground">
                      {[
                        person.name,
                        person.email,
                        ...(person.aliases ?? []).map((alias) =>
                          typeof alias === "string"
                            ? alias
                            : alias && typeof alias === "object"
                              ? Object.values(alias)
                                  .filter((value) => typeof value === "string")
                                  .join(" · ")
                              : "",
                        ),
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </small>
                  </span>
                </label>
              ))}
          </div>
          <a
            className="text-primary underline"
            href={navigation?.href(
              {
                group: "repository",
                item: "code-insights",
                id: insight,
                mode: "view",
              },
              { tab: "authors" },
            )}
          >
            {text("aliases")}
          </a>
        </fieldset>
      ) : (
        <fieldset className="space-y-4">
          <legend className="font-medium">{text("applications")}</legend>
          {options.applications.map((app) => {
            const selected = apps.find((row) => row.application_id === app.id);
            return (
              <div key={app.id} className="space-y-2 border-b pb-4">
                <label className="flex min-h-11 items-center gap-2 font-medium">
                  <Checkbox
                    checked={!!selected}
                    disabled={readOnly}
                    onCheckedChange={(checked) =>
                      onPatch({
                        applications: checked
                          ? [
                              ...apps,
                              {
                                application_id: app.id,
                                repositories: (app.repositories ?? []).filter(
                                  (repo) =>
                                    options.repositories.some(
                                      (allowed) =>
                                        repoKey(allowed) === repoKey(repo),
                                    ),
                                ),
                              },
                            ]
                          : apps.filter((row) => row.application_id !== app.id),
                      })
                    }
                  />
                  {app.name}
                </label>
                {selected && (
                  <div className="flex flex-wrap gap-x-6 gap-y-2 pl-6">
                    {options.repositories.map((repo) => (
                      <label
                        key={repoKey(repo)}
                        className="flex min-h-11 max-w-full items-center gap-2 break-all"
                      >
                        <Checkbox
                          checked={selected.repositories.some(
                            (row) => repoKey(row) === repoKey(repo),
                          )}
                          disabled={readOnly}
                          onCheckedChange={(checked) =>
                            onPatch({
                              applications: apps.map((row) =>
                                row.application_id !== app.id
                                  ? row
                                  : {
                                      ...row,
                                      repositories: checked
                                        ? [...row.repositories, repo]
                                        : row.repositories.filter(
                                            (item) =>
                                              repoKey(item) !== repoKey(repo),
                                          ),
                                    },
                              ),
                            })
                          }
                        />
                        {repo.repository}
                      </label>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </fieldset>
      )}
    </div>
  );
}
