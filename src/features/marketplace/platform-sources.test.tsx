import { beforeEach, describe, expect, it, vi } from "vitest";
import { setLocale } from "../../shared/i18n/locale";
import { platformMarketplaceForm, platformMarketplaceList, type PlatformSource } from "./platform-sources";
import { marketplaceLists, type MarketEntry } from "./resource-config";

beforeEach(() => setLocale("en-US", false));

const source = (patch: Partial<PlatformSource> = {}): PlatformSource => ({
  id: "1", name: "Official", trust: "official", enabled: true, sync_interval_hours: 6, provider: "github",
  api_base: "https://api.github.com", project: "ferry/templates", organization: "", branch: "main",
  index_path: "marketplace/index.json", state: "idle", last_error: "", synced_at: 0, next_sync_at: 0,
  entry_count: 3, version: 1, ...patch,
});

describe("platform template sources", () => {
  it("uses the platform endpoint and disables sync while syncing", () => {
    const list = platformMarketplaceList();
    expect(list.endpoint).toBe("/platform/marketplace-sources");
    const sync = list.actions!.find(a => a.labelKey === "resource-domain:market.syncNow")!;
    expect(sync.disabled!(source({ state: "syncing" }))).toBe(true);
    expect(sync.disabled!(source())).toBe(false);
    const trust = list.columns.find(c => c.key === "trust")!;
    expect(trust.render!(source({ trust: "community" }))).toBe("Community");
  });
  it("shows the organization only for Codeup and bounds the interval", () => {
    const fields = platformMarketplaceForm().sections.flatMap(s => s.fields);
    const organization = fields.find(f => f.key === "organization")!;
    expect(organization.when!({ provider: "codeup" })).toBe(true);
    expect(organization.when!({ provider: "github" })).toBe(false);
    const interval = fields.find(f => f.key === "sync_interval_hours")!;
    expect([interval.min, interval.max]).toEqual([1, 168]);
    expect((fields.find(f => f.key === "trust")!.options as { value: string }[]).map(o => o.value)).toEqual(["official", "community"]);
  });
});

describe("workspace market list", () => {
  const entry = (patch: Partial<MarketEntry> = {}): MarketEntry => ({
    id: "9", source_id: "1", source_name: "Official", trust: "official", entry_key: "web", name: "Website",
    description: "", content: "{}", parameters: "[]", version: 1, package_version: "1.0.0", kind: "app",
    compatible: true, incompatible_reason: null, readme: "", files: [], verification: "none", verified_drivers: [], ...patch,
  });
  const list = () => marketplaceLists({ workspace: "w", permissions: new Set(["marketplace.install"]), navigate: vi.fn() } as never).marketplace!;
  it("disables import for incompatible versions with the reason", () => {
    const install = list().actions!.find(a => a.labelKey === "resource-domain:template.importApp")!;
    const row = entry({ compatible: false, incompatible_reason: "Needs a newer Ferry" });
    expect(install.disabled!(row)).toBe(true);
    expect(install.disabledReason!(row)).toBe("Needs a newer Ferry");
    expect(install.disabled!(entry())).toBe(false);
  });
  it("shows a dash instead of the synthetic 0.0.0 for legacy index entries", () => {
    const column = list().columns.find(c => c.key === "package_version")!;
    const legacy = JSON.stringify(column.render!(entry({ package_version: "0.0.0" })));
    expect(legacy).toContain('"—"');
    expect(legacy).not.toContain("0.0.0");
    expect(JSON.stringify(column.render!(entry({ package_version: "1.2.0" })))).toContain('"1.2.0"');
  });
  it("marks official verified versions and community declarations next to the version", () => {
    const column = list().columns.find(c => c.key === "package_version")!;
    const text = (row: MarketEntry) => JSON.stringify(column.render!(row));
    const verified = text(entry({ verification: "verified", verified_drivers: ["docker", "compose"] }));
    expect(verified).toContain("Verified");
    expect(verified).toContain("docker, compose");
    expect(text(entry({ trust: "community", verification: "declared", verified_drivers: ["docker"] }))).toContain("Author declared");
    expect(text(entry())).not.toContain("Verified");
  });
  it("labels each source kind and shows the version", () => {
    const columns = list().columns;
    expect(columns.some(c => c.key === "package_version")).toBe(true);
    const sourceColumn = columns.find(c => c.key === "source_name")!;
    for (const [trust, label] of [["official", "Official"], ["community", "Community"], ["workspace", "This workspace"]] as const) {
      const text = JSON.stringify(sourceColumn.render!(entry({ trust })));
      expect(text).toContain(label);
    }
    expect(list().filters!.some(f => f.key === "trust")).toBe(true);
  });
});
