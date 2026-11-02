export const CUSTOM_VARIABLE_KEYS = [
  '--background',
  '--foreground',
  '--card',
  '--card-foreground',
  '--popover',
  '--popover-foreground',
  '--primary',
  '--primary-foreground',
  '--secondary',
  '--secondary-foreground',
  '--muted',
  '--muted-foreground',
  '--accent',
  '--accent-foreground',
  '--destructive',
  '--destructive-foreground',
  '--border',
  '--input',
  '--ring',
  '--success',
  '--success-foreground',
  '--warning',
  '--warning-foreground',
  '--font-body',
  '--font-heading',
  '--radius',
  '--border-width',
  '--shadow-sm',
  '--shadow-md',
  '--shadow-lg',
];
const CUSTOM_VARIABLE_KEY_SET = new Set<string>(CUSTOM_VARIABLE_KEYS);
const COLOR_KEYS = new Set(
  CUSTOM_VARIABLE_KEYS.filter((key) =>
    !['--font-body', '--font-heading', '--radius', '--border-width', '--shadow-sm', '--shadow-md', '--shadow-lg'].includes(key)
  )
);
const FONT_VALUES = new Set([
  'var(--font-sans), system-ui, sans-serif',
  'Georgia, "Times New Roman", serif',
  '"Courier New", monospace',
  'system-ui, sans-serif',
]);
const SHADOW_VALUES = new Set([
  'none',
  '0 1px 2px 0 rgb(0 0 0 / 0.05)',
  '0 4px 6px -1px rgb(0 0 0 / 0.08), 0 2px 4px -2px rgb(0 0 0 / 0.04)',
  '0 10px 15px -3px rgb(0 0 0 / 0.08), 0 4px 6px -4px rgb(0 0 0 / 0.04)',
  '0 1px 2px 0 rgb(0 0 0 / 0.08)',
  '0 8px 16px -4px rgb(0 0 0 / 0.12), 0 4px 8px -4px rgb(0 0 0 / 0.06)',
  '0 20px 25px -5px rgb(0 0 0 / 0.12), 0 8px 10px -6px rgb(0 0 0 / 0.06)',
  '3px 3px 0 0 rgb(0 0 0 / 1)',
  '5px 5px 0 0 rgb(0 0 0 / 1)',
  '8px 8px 0 0 rgb(0 0 0 / 1)',
  '0 1px 2px 0 rgb(99 102 241 / 0.2)',
  '0 4px 18px rgb(99 102 241 / 0.35)',
  '0 10px 25px -5px rgb(99 102 241 / 0.4)',
]);

export function isValidCustomThemeVariable(key: string, value: string) {
  if (!CUSTOM_VARIABLE_KEY_SET.has(key)) return false;
  if (COLOR_KEYS.has(key)) {
    return (
      /^#[0-9A-Fa-f]{6}$/.test(value) ||
      /^rgb\(\d{1,3} \d{1,3} \d{1,3}\)$/.test(value) ||
      /^oklch\([0-9.]+ [0-9.]+ [0-9.]+\)$/.test(value)
    );
  }
  if (key === '--font-body' || key === '--font-heading') return FONT_VALUES.has(value);
  if (key === '--radius') return /^(0|0\.\d{1,3}|1(\.\d{1,3})?|2(\.0{1,3})?)rem$/.test(value);
  if (key === '--border-width') return /^[0-4]px$/.test(value);
  if (key.startsWith('--shadow-')) return SHADOW_VALUES.has(value);
  return false;
}

export function sanitizeCustomThemeVariables(value: unknown): ThemeVariables {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  const sanitized: ThemeVariables = {};
  Object.entries(value as Record<string, unknown>).forEach(([key, rawValue]) => {
    if (typeof rawValue === 'string' && isValidCustomThemeVariable(key, rawValue)) {
      sanitized[key] = rawValue;
    }
  });
  return sanitized;
}

export type ThemeVariables = Record<string, string>;
