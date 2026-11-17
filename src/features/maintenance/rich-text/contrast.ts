/**
 * Readable text on a stored highlight colour. Stored colours do not follow the
 * theme, so text without its own colour would otherwise inherit the theme's
 * foreground (e.g. light text on yellow in dark mode). Used only for display;
 * the saved HTML keeps exactly what the author chose.
 */
const DARK = "#111111";
const LIGHT = "#ffffff";

/** sRGB channels 0–255 of a hex or rgb()/rgba() colour; undefined for anything else. */
export function parseColor(value: string): [number, number, number, number] | undefined {
  const text = value.trim().toLowerCase();
  const hex = /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/.exec(text)?.[1];
  if (hex) {
    const full = hex.length <= 4 ? [...hex].map(digit => digit + digit).join("") : hex;
    const [r, g, b, a = 255] = full.match(/../g)!.map(pair => parseInt(pair, 16));
    return [r!, g!, b!, a / 255];
  }
  const rgb = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:\s*[,/]\s*([\d.]+%?))?\s*\)$/.exec(text);
  if (!rgb) return undefined;
  const alpha = rgb[4] === undefined ? 1 : rgb[4].endsWith("%") ? parseFloat(rgb[4]) / 100 : parseFloat(rgb[4]);
  return [Number(rgb[1]), Number(rgb[2]), Number(rgb[3]), alpha];
}

/** WCAG 2 relative luminance of an sRGB colour. */
export function luminance([r, g, b]: readonly number[]): number {
  const linear = (channel: number) => {
    const c = Math.min(255, Math.max(0, channel)) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(r!) + 0.7152 * linear(g!) + 0.0722 * linear(b!);
}

const ratio = (a: number, b: number) => (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);

/**
 * Near-black or white, whichever contrasts more with `background`; undefined
 * when the colour is unknown or mostly transparent (the theme text then shows).
 */
export function readableText(background: string): string | undefined {
  const color = parseColor(background);
  if (!color || color[3] < 0.5) return undefined;
  const lum = luminance(color);
  return ratio(lum, luminance(parseColor(DARK)!)) >= ratio(lum, 1) ? DARK : LIGHT;
}
