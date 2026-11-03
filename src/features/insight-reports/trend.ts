import type { Row } from "./ReportScope";

/** Group immutable snapshot evidence using the same local dates as the report. */
export function reportTrend(snapshot: Row): Row[] {
  const formatter = new Intl.DateTimeFormat("en-CA", {
    timeZone: String(snapshot.timezone || "UTC"),
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  });
  const dates = new Map<string, Map<string, Row>>();
  for (const commit of (snapshot.commits ?? []) as Row[]) {
    if (
      typeof commit.author_date !== "number" ||
      !Number.isFinite(commit.author_date)
    )
      continue;
    const parts = Object.fromEntries(
      formatter
        .formatToParts(commit.author_date)
        .map((part) => [part.type, part.value]),
    );
    const date = `${parts.year}-${parts.month}-${parts.day}`;
    const key = String(
      commit.author_key ?? commit.author_email ?? commit.author_name ?? "",
    );
    const people = dates.get(date) ?? new Map<string, Row>();
    const person = people.get(key) ?? {
      authorKey: key,
      nickname: commit.nickname,
      name: commit.author_name,
      email: commit.author_email,
      commits: 0,
      effectives: 0,
    };
    person.commits = Number(person.commits) + 1;
    person.effectives =
      Number(person.effectives) + Number(commit.effectives ?? 0);
    people.set(key, person);
    dates.set(date, people);
  }
  return ((snapshot.trend ?? []) as Row[]).map((point) => {
    const known = dates.get(String(point.date));
    const authors = Array.isArray(point.authors)
      ? (point.authors as Row[]).map((author) => ({
          ...known?.get(String(author.key ?? author.authorKey ?? "")),
          ...author,
        }))
      : [...(known?.values() ?? [])];
    return {
      ...point,
      authors,
      contributors:
        typeof point.contributors === "number"
          ? point.contributors
          : authors.length,
    };
  });
}
