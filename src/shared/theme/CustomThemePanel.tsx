import { useMemo,useState } from 'react';
import { useTranslation } from 'react-i18next';
import '../i18n/i18n';

import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { useTheme } from './theme';
import { sanitizeCustomThemeVariables } from './theme-validation';

import { Copy,RotateCcw,Shuffle,Upload } from 'lucide-react';
import { Badge } from '../ui/badge';
import { Tabs,TabsContent,TabsList,TabsTrigger } from '../ui/tabs';

const colorKeys = [
  { key: '--background', labelKey: 'background' },
  { key: '--foreground', labelKey: 'foreground' },
  { key: '--card', labelKey: 'card' },
  { key: '--card-foreground', labelKey: 'cardForeground' },
  { key: '--popover', labelKey: 'popover' },
  { key: '--popover-foreground', labelKey: 'popoverForeground' },
  { key: '--primary', labelKey: 'primary' },
  { key: '--primary-foreground', labelKey: 'primaryForeground' },
  { key: '--secondary', labelKey: 'secondary' },
  { key: '--secondary-foreground', labelKey: 'secondaryForeground' },
  { key: '--muted', labelKey: 'muted' },
  { key: '--muted-foreground', labelKey: 'mutedForeground' },
  { key: '--accent', labelKey: 'accent' },
  { key: '--accent-foreground', labelKey: 'accentForeground' },
  { key: '--destructive', labelKey: 'destructive' },
  { key: '--destructive-foreground', labelKey: 'destructiveForeground' },
  { key: '--success', labelKey: 'success' },
  { key: '--warning', labelKey: 'warning' },
  { key: '--border', labelKey: 'border' },
  { key: '--input', labelKey: 'input' },
  { key: '--ring', labelKey: 'ring' },
];

const fontOptions = [
  { value: 'var(--font-sans), system-ui, sans-serif', labelKey: 'sans' },
  { value: 'Georgia, "Times New Roman", serif', labelKey: 'serif' },
  { value: '"Courier New", monospace', labelKey: 'mono' },
  { value: 'system-ui, sans-serif', labelKey: 'system' },
];

const shadowPresets = [
  { value: 'none', labelKey: 'none' },
  {
    value: 'soft',
    labelKey: 'soft',
    sm: '0 1px 2px 0 rgb(0 0 0 / 0.05)',
    md: '0 4px 6px -1px rgb(0 0 0 / 0.08), 0 2px 4px -2px rgb(0 0 0 / 0.04)',
    lg: '0 10px 15px -3px rgb(0 0 0 / 0.08), 0 4px 6px -4px rgb(0 0 0 / 0.04)',
  },
  {
    value: 'medium',
    labelKey: 'medium',
    sm: '0 1px 2px 0 rgb(0 0 0 / 0.08)',
    md: '0 8px 16px -4px rgb(0 0 0 / 0.12), 0 4px 8px -4px rgb(0 0 0 / 0.06)',
    lg: '0 20px 25px -5px rgb(0 0 0 / 0.12), 0 8px 10px -6px rgb(0 0 0 / 0.06)',
  },
  {
    value: 'hard',
    labelKey: 'hard',
    sm: '3px 3px 0 0 rgb(0 0 0 / 1)',
    md: '5px 5px 0 0 rgb(0 0 0 / 1)',
    lg: '8px 8px 0 0 rgb(0 0 0 / 1)',
  },
  {
    value: 'glow',
    labelKey: 'glow',
    sm: '0 1px 2px 0 rgb(99 102 241 / 0.2)',
    md: '0 4px 18px rgb(99 102 241 / 0.35)',
    lg: '0 10px 25px -5px rgb(99 102 241 / 0.4)',
  },
];

/**
 * Convert any CSS color (oklch / hex / rgb / named) to #rrggbb using the browser's own
 * color resolution, so the native color input can both PREVIEW the current value and
 * round-trip a picked one. Returns '' when it can't resolve (SSR or invalid value).
 */
function cssColorToHex(value: string | undefined): string {
  const v = value?.trim();
  if (!v) return '';
  if (/^#[0-9a-f]{6}$/i.test(v)) return v.toLowerCase();
  if (typeof document === 'undefined') return '';
  const probe = document.createElement('span');
  probe.style.color = v;
  document.body.appendChild(probe);
  const rgb = getComputedStyle(probe).color;
  document.body.removeChild(probe);
  const m = /rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)/.exec(rgb);
  if (!m) return '';
  const toHex = (n: string) => Math.round(Number(n)).toString(16).padStart(2, '0');
  return `#${toHex(m[1]!)}${toHex(m[2]!)}${toHex(m[3]!)}`;
}

/** HSL (h:0-360, s/l:0-100) → #rrggbb. */
function hslToHex(h: number, s: number, l: number): string {
  s /= 100;
  l /= 100;
  const k = (n: number) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => {
    const c = l - a * Math.max(-1, Math.min(k(n) - 3, 9 - k(n), 1));
    return Math.round(255 * c)
      .toString(16)
      .padStart(2, '0');
  };
  return `#${f(0)}${f(8)}${f(4)}`;
}

/**
 * Generate a COORDINATED (not noisy) light+dark palette: one random base hue drives the
 * neutrals (slightly hue-tinted), an analogous/near-complementary accent, and fixed
 * semantic hues (red/green/amber). Lightness/saturation are constrained for readable
 * contrast, so every roll looks intentional.
 */
function generateHarmoniousPalette(): {
  light: Record<string, string>;
  dark: Record<string, string>;
} {
  const base = Math.floor(Math.random() * 360);
  const accent = (base + 150 + Math.floor(Math.random() * 60)) % 360;
  // White text reads on a mid-light primary unless the hue is intrinsically bright (yellow/cyan).
  const brightHue = base > 40 && base < 200;
  const light: Record<string, string> = {
    '--background': hslToHex(base, 22, 98),
    '--foreground': hslToHex(base, 25, 12),
    '--card': hslToHex(base, 18, 100),
    '--card-foreground': hslToHex(base, 25, 12),
    '--popover': hslToHex(base, 18, 100),
    '--popover-foreground': hslToHex(base, 25, 12),
    '--primary': hslToHex(base, 68, brightHue ? 42 : 52),
    '--primary-foreground': hslToHex(base, 30, 98),
    '--secondary': hslToHex(base, 25, 94),
    '--secondary-foreground': hslToHex(base, 25, 18),
    '--muted': hslToHex(base, 22, 95),
    '--muted-foreground': hslToHex(base, 12, 42),
    '--accent': hslToHex(accent, 55, 92),
    '--accent-foreground': hslToHex(accent, 40, 20),
    '--destructive': hslToHex(2, 75, 52),
    '--destructive-foreground': '#ffffff',
    '--border': hslToHex(base, 20, 88),
    '--input': hslToHex(base, 20, 88),
    '--ring': hslToHex(base, 68, 52),
    '--success': hslToHex(145, 55, 40),
    '--warning': hslToHex(40, 80, 46),
  };
  const dark: Record<string, string> = {
    '--background': hslToHex(base, 28, 8),
    '--foreground': hslToHex(base, 15, 96),
    '--card': hslToHex(base, 25, 11),
    '--card-foreground': hslToHex(base, 15, 96),
    '--popover': hslToHex(base, 25, 11),
    '--popover-foreground': hslToHex(base, 15, 96),
    '--primary': hslToHex(base, 70, 62),
    '--primary-foreground': hslToHex(base, 30, 10),
    '--secondary': hslToHex(base, 20, 18),
    '--secondary-foreground': hslToHex(base, 15, 92),
    '--muted': hslToHex(base, 18, 16),
    '--muted-foreground': hslToHex(base, 12, 64),
    '--accent': hslToHex(accent, 35, 22),
    '--accent-foreground': hslToHex(accent, 30, 90),
    '--destructive': hslToHex(2, 70, 60),
    '--destructive-foreground': hslToHex(0, 0, 10),
    '--border': hslToHex(base, 18, 20),
    '--input': hslToHex(base, 18, 22),
    '--ring': hslToHex(base, 70, 62),
    '--success': hslToHex(145, 55, 55),
    '--warning': hslToHex(45, 85, 60),
  };
  return { light, dark };
}

export function CustomThemePanel() {
  const { t: translate } = useTranslation('theme');
  const t = (key: string) => translate(key, { keySeparator: false });
  const { customLight, customDark, setCustomVariable, resetCustomVariables } = useTheme();
  const [mode, setMode] = useState<'light' | 'dark'>('light');
  const [importText, setImportText] = useState('');
  const [showImport, setShowImport] = useState(false);
  const [feedback, setFeedback] = useState('');
  const [colorDrafts, setColorDrafts] = useState<Record<string, string>>({});

  const custom = mode === 'light' ? customLight : customDark;
  const { resolved: resolvedTheme } = useTheme();

  // The preview must match the CURRENT document light/dark mode (not the editing tab), so the
  // surface's --background and inherited --foreground come from the same palette — otherwise a
  // light-tab preview inside a dark app gives e.g. a white button bg with white (dark-mode) text.
  const previewStyle = useMemo(() => {
    const src = resolvedTheme === 'dark' ? customDark : customLight;
    return { ...src } as Record<string, string>;
  }, [resolvedTheme, customLight, customDark]);

  const handleColorChange = (themeMode: 'light' | 'dark', key: string, value: string) => {
    // The native picker yields #rrggbb; store it directly so it round-trips to the swatch.
    setCustomVariable(themeMode, key, value);
    setColorDrafts((prev) => ({ ...prev, [`${themeMode}:${key}`]: value }));
  };

  /** Hex shown by the native color swatch: the override if set, else the computed live value. */
  const getSwatchHex = (themeMode: 'light' | 'dark', key: string) => {
    const override = themeMode === 'light' ? customLight[key] : customDark[key];
    const fromOverride = cssColorToHex(override);
    if (fromOverride) return fromOverride;
    if (typeof document !== 'undefined') {
      const computed = getComputedStyle(document.documentElement).getPropertyValue(key);
      const fromComputed = cssColorToHex(computed);
      if (fromComputed) return fromComputed;
    }
    return '#000000';
  };

  const handleColorTextChange = (themeMode: 'light' | 'dark', key: string, value: string) => {
    const draftKey = `${themeMode}:${key}`;
    setColorDrafts((prev) => ({ ...prev, [draftKey]: value }));
    setCustomVariable(themeMode, key, value);
  };

  const handleColorTextBlur = (themeMode: 'light' | 'dark', key: string) => {
    const draftKey = `${themeMode}:${key}`;
    setColorDrafts((prev) => {
      if (prev[draftKey] === undefined) return prev;
      const next = { ...prev };
      delete next[draftKey];
      return next;
    });
  };

  const getColorTextValue = (themeMode: 'light' | 'dark', key: string) => {
    const draft = colorDrafts[`${themeMode}:${key}`];
    return draft ?? (themeMode === 'light' ? customLight[key] : customDark[key]) ?? '';
  };

  const handleRandomize = () => {
    const { light, dark } = generateHarmoniousPalette();
    Object.entries(light).forEach(([k, v]) => setCustomVariable('light', k, v));
    Object.entries(dark).forEach(([k, v]) => setCustomVariable('dark', k, v));
    setColorDrafts({});
  };

  const handleFontChange = (value: string) => {
    setCustomVariable(mode, '--font-body', value);
    setCustomVariable(mode, '--font-heading', value);
  };

  const handleRadiusChange = (value: string) => {
    setCustomVariable(mode, '--radius', `${value}rem`);
  };

  const handleBorderWidthChange = (value: string) => {
    setCustomVariable(mode, '--border-width', `${value}px`);
  };

  const applyShadowPreset = (presetValue: string) => {
    const preset = shadowPresets.find((p) => p.value === presetValue);
    if (!preset || preset.value === 'none') {
      setCustomVariable(mode, '--shadow-sm', 'none');
      setCustomVariable(mode, '--shadow-md', 'none');
      setCustomVariable(mode, '--shadow-lg', 'none');
      return;
    }
    setCustomVariable(mode, '--shadow-sm', preset.sm ?? 'none');
    setCustomVariable(mode, '--shadow-md', preset.md ?? 'none');
    setCustomVariable(mode, '--shadow-lg', preset.lg ?? 'none');
  };

  const exportJson = () => {
    return JSON.stringify({ light: customLight, dark: customDark }, null, 2);
  };

  const handleCopy = async () => {
    try { await navigator.clipboard.writeText(exportJson()); setFeedback(t('copied')); }
    catch { setFeedback(t('copyFailed')); }
  };

  const handleImport = () => {
    try {
      const parsed = JSON.parse(importText) as { light?: Record<string, string>; dark?: Record<string, string> };
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed) || (!parsed.light && !parsed.dark)) throw new Error('invalid');
      if (parsed.light) {
        Object.entries(sanitizeCustomThemeVariables(parsed.light)).forEach(([key, value]) =>
          setCustomVariable('light', key, value)
        );
      }
      if (parsed.dark) {
        Object.entries(sanitizeCustomThemeVariables(parsed.dark)).forEach(([key, value]) =>
          setCustomVariable('dark', key, value)
        );
      }
      setImportText('');
      setShowImport(false);
      setColorDrafts({});
      setFeedback(t('imported'));
    } catch {
      setFeedback(t('importFailed'));
    }
  };

  return (
    <div className="space-y-6">
      <Tabs value={mode} onValueChange={(v) => setMode(v as 'light' | 'dark')}>
        <TabsList className="grid w-full grid-cols-2">
          <TabsTrigger value="light">{t('mode.light')}</TabsTrigger>
          <TabsTrigger value="dark">{t('mode.dark')}</TabsTrigger>
        </TabsList>

        {(['light', 'dark'] as const).map((m) => (
          <TabsContent key={m} value={m} className="space-y-4">
            <div className="grid gap-4 sm:grid-cols-2">
              {colorKeys.map(({ key, labelKey }) => {
                const label = t(`customPanel.colors.${labelKey}`);
                const textValue = getColorTextValue(m, key);

                return (
                <div key={key} className="flex items-center justify-between gap-2">
                  <label className="text-sm">{label}</label>
                  <div className="flex items-center gap-2">
                    <Input
                      type="text"
                      aria-label={label}
                      value={textValue}
                      onChange={(e) => handleColorTextChange(m, key, e.target.value)}
                      onBlur={() => handleColorTextBlur(m, key)}
                      className="h-8 w-32 text-xs"
                      placeholder={t('customPanel.auto')}
                    />
                    <input
                      type="color"
                      value={getSwatchHex(m, key)}
                      onChange={(e) => handleColorChange(m, key, e.target.value)}
                      className="h-8 w-8 cursor-pointer rounded border-0 p-0"
                      aria-label={label}
                    />
                  </div>
                </div>
              )})}
            </div>

            <div className="space-y-3 rounded-md border p-4">
              <h4 className="text-sm font-semibold">{t('customPanel.typography')}</h4>
              <div className="flex items-center justify-between gap-2">
                <label className="text-sm">{t('customPanel.fontFamily')}</label>
                <select
                  aria-label={t("customPanel.fontFamily")}
                  value={custom['--font-body'] ?? fontOptions[0]!.value}
                  onChange={(e) => handleFontChange(e.target.value)}
                  className="h-8 rounded-md border bg-background px-2 text-sm"
                >
                  {fontOptions.map((opt) => (
                    <option key={opt.value} value={opt.value}>
                      {t(`customPanel.fonts.${opt.labelKey}`)}
                    </option>
                  ))}
                </select>
              </div>

              <div className="flex items-center justify-between gap-2">
                <label className="text-sm">
                  {t('customPanel.radius')} ({parseFloat(custom['--radius'] ?? '0.875')}rem)
                </label>
                <input
                  type="range"
                  aria-label={t("customPanel.radius")}
                  min="0"
                  max="2"
                  step="0.125"
                  value={parseFloat(custom['--radius'] ?? '0.875')}
                  onChange={(e) => handleRadiusChange(e.target.value)}
                  className="w-32"
                />
              </div>

              <div className="flex items-center justify-between gap-2">
                <label className="text-sm">
                  {t('customPanel.borderWidth')} ({parseInt(custom['--border-width'] ?? '1', 10)}px)
                </label>
                <input
                  type="range"
                  aria-label={t("customPanel.borderWidth")}
                  min="0"
                  max="4"
                  step="1"
                  value={parseInt(custom['--border-width'] ?? '1', 10)}
                  onChange={(e) => handleBorderWidthChange(e.target.value)}
                  className="w-32"
                />
              </div>

              <div className="flex items-center justify-between gap-2">
                <label className="text-sm">{t('customPanel.shadowPreset')}</label>
                <select
                  aria-label={t("customPanel.shadowPreset")}
                  onChange={(e) => applyShadowPreset(e.target.value)}
                  className="h-8 rounded-md border bg-background px-2 text-sm"
                  defaultValue=""
                >
                  <option value="" disabled>{t('customPanel.choosePreset')}</option>
                  {shadowPresets.map((p) => (
                    <option key={p.value} value={p.value}>
                      {t(`customPanel.shadows.${p.labelKey}`)}
                    </option>
                  ))}
                </select>
              </div>
            </div>
          </TabsContent>
        ))}
      </Tabs>

      <div className="space-y-3 rounded-md border bg-background p-4 text-foreground" style={previewStyle}>
        <h4 className="text-sm font-semibold">{t('customPanel.preview')}</h4>
        <div className="flex flex-wrap items-center gap-3">
          <Button className="btn-primary">{t('customPanel.primaryButton')}</Button>
          <Button variant="outline">{t('customPanel.outlineButton')}</Button>
          <Input placeholder={t('customPanel.inputPlaceholder')} className="w-40" />
          <Badge>{t('customPanel.badge')}</Badge>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button variant="outline" size="sm" onClick={handleRandomize}>
          <Shuffle className="mr-1 size-3.5" />
          {t('customPanel.randomize')}
        </Button>
        <Button variant="outline" size="sm" onClick={() => { resetCustomVariables(); setColorDrafts({}); setFeedback(t('resetDone')); }}>
          <RotateCcw className="mr-1 size-3.5" />
          {t('customPanel.reset')}
        </Button>
        <Button variant="outline" size="sm" onClick={handleCopy}>
          <Copy className="mr-1 size-3.5" />
          {t('customPanel.copyJson')}
        </Button>
        <Button variant="outline" size="sm" onClick={() => setShowImport((v) => !v)}>
          <Upload className="mr-1 size-3.5" />
          {t('customPanel.import')}
        </Button>
      </div>

      {feedback && <p role="status" className="text-sm text-muted-foreground">{feedback}</p>}

      {showImport && (
        <div className="space-y-2">
          <textarea aria-label={t("customPanel.importPlaceholder")}
            value={importText}
            onChange={(e) => setImportText(e.target.value)}
            placeholder={t('customPanel.importPlaceholder')}
            className="min-h-[120px] w-full rounded-md border bg-transparent p-2 text-sm"
          />
          <Button size="sm" onClick={handleImport}>{t('customPanel.applyImport')}</Button>
        </div>
      )}
    </div>
  );
}
