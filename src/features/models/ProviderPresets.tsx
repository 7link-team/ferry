import { useMemo, useState } from "react";
import { ExternalLink, Search } from "lucide-react";
import { commonText } from "../../shared/i18n/i18n";
import { Input } from "../../shared/ui/input";
import catalog from "./provider-presets.json";

export type ProviderPreset = {
  name: string;
  category: string;
  protocol: "openai" | "anthropic";
  base_url: string;
  models: string[];
  website?: string;
  api_key_url?: string;
};

export const providerPresets = catalog.presets as ProviderPreset[];
const categories = ["all", "official", "cn_official", "aggregator", "third_party"];
const t = (key: string) => commonText(`models:${key}`);

/** Matches a saved connection back to the preset it came from, if any. */
export function matchPreset(protocol: unknown, baseUrl: unknown) {
  const url = String(baseUrl ?? "").replace(/\/+$/, "");
  return providerPresets.find((p) => p.protocol === protocol && p.base_url === url);
}

/** CC Switch-style provider chooser: category filter, search and preset chips. */
export function ProviderPresetPicker({
  selected,
  onSelect,
}: {
  selected: ProviderPreset | undefined;
  onSelect: (preset: ProviderPreset | null) => void;
}) {
  const [category, setCategory] = useState("all");
  const [query, setQuery] = useState("");
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return providerPresets.filter(
      (p) =>
        (category === "all" || p.category === category) &&
        (!q || p.name.toLowerCase().includes(q) || p.base_url.toLowerCase().includes(q)),
    );
  }, [category, query]);
  const chip = (active: boolean) =>
    `inline-flex min-h-9 max-w-full items-center rounded-full border px-3 text-sm transition-colors ${
      active
        ? "border-primary bg-primary text-primary-foreground"
        : "border-input bg-background hover:bg-accent hover:text-accent-foreground"
    }`;
  return (
    <section className="space-y-3 rounded-lg border bg-muted/30 p-3" aria-label={t("presets")}>
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="text-sm font-medium">{t("presets")}</span>
        <div role="tablist" className="flex min-w-0 flex-wrap gap-1">
          {categories.map((value) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={category === value}
              className={`min-h-8 rounded-md px-2 text-xs ${
                category === value ? "bg-background font-medium shadow-sm" : "text-muted-foreground"
              }`}
              onClick={() => setCategory(value)}
            >
              {t(`category.${value}`)}
            </button>
          ))}
        </div>
        <label className="relative ml-auto w-full min-w-0 sm:w-56">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            aria-label={t("presetSearch")}
            placeholder={t("presetSearch")}
            className="min-h-9 pl-8"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>
      </div>
      <div className="flex max-h-48 min-w-0 flex-wrap gap-2 overflow-y-auto">
        <button type="button" aria-pressed={!selected} className={chip(!selected)} onClick={() => onSelect(null)}>
          {t("presetCustom")}
        </button>
        {visible.map((preset) => (
          <button
            key={preset.name}
            type="button"
            aria-pressed={selected?.name === preset.name}
            className={chip(selected?.name === preset.name)}
            title={preset.base_url}
            onClick={() => onSelect(preset)}
          >
            <span className="truncate">{preset.name}</span>
          </button>
        ))}
        {!visible.length && <p className="text-sm text-muted-foreground">{t("presetNone")}</p>}
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1 text-sm text-muted-foreground">
        <span>{t("presetHelp")}</span>
        {selected?.website && (
          <a className="inline-flex items-center gap-1 text-primary hover:underline" href={selected.website} target="_blank" rel="noreferrer">
            {t("presetWebsite")} <ExternalLink className="size-3.5" />
          </a>
        )}
        {selected?.api_key_url && (
          <a className="inline-flex items-center gap-1 text-primary hover:underline" href={selected.api_key_url} target="_blank" rel="noreferrer">
            {t("presetApiKey")} <ExternalLink className="size-3.5" />
          </a>
        )}
      </div>
    </section>
  );
}
