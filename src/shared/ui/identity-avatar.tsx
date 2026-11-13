import { useEffect, useMemo, useRef, useState } from "react";
import { createAvatar } from "@dicebear/core";
import * as micah from "@dicebear/micah";
import * as adventurer from "@dicebear/adventurer";
import * as notionists from "@dicebear/notionists";
import * as lorelei from "@dicebear/lorelei";
import * as pixelArt from "@dicebear/pixel-art";
import { commonText as text, useCommonTranslation } from "../i18n/i18n";
import { Button } from "./button";
import { AutoField } from "../forms/auto-form";
import type { AvatarValue } from "../api/api";
export type { AvatarValue } from "../api/api";
const t = (key: string) => text(`identity:${key}`);
const avatarStyles = ["portrait", "adventurer", "notionists", "lorelei", "pixel-art", "pixel", "initials", "emoji"] as const;
/** Bundled, deterministic artwork; never sends identity or seeds to a remote avatar service. */
export function illustrationSource(style: AvatarValue["style"], seed: string, gender?: AvatarValue["gender"]) {
  const options = { seed, size: 128 };
  // Appearance presets, not inferred identity. Keep unannotated historical avatars unchanged.
  const hairKind = gender === "male" ? "short" : "long";
  const variantHair = (numbers: number[]) => numbers.map(number => `variant${String(number).padStart(2, "0")}`);
  const namedHair = (count: number) => Array.from({ length: count }, (_, index) => `${hairKind}${String(index + 1).padStart(2, "0")}`);
  switch (style) {
    case "adventurer": return createAvatar(adventurer, { ...options, ...(gender && gender !== "neutral" ? { hair: namedHair(gender === "male" ? 19 : 26) as NonNullable<adventurer.Options["hair"]> } : {}) }).toDataUri();
    case "notionists": return createAvatar(notionists, { ...options, ...(gender ? { beardProbability: gender === "male" ? 25 : 0 } : {}), ...(gender && gender !== "neutral" ? { hair: variantHair(gender === "male" ? [3, 5, 7, 9, 12, 18, 21, 22, 27, 31, 38, 40, 44, 49, 50, 53, 55, 60] : [1, 2, 8, 10, 11, 16, 17, 23, 28, 30, 36, 39, 41, 45, 46, 47, 57, 58, 61, 62, 63]) as NonNullable<notionists.Options["hair"]> } : {}) }).toDataUri();
    case "lorelei": return createAvatar(lorelei, { ...options, ...(gender ? { beardProbability: gender === "male" ? 25 : 0 } : {}), ...(gender && gender !== "neutral" ? { hair: variantHair(gender === "male" ? [1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 22, 25, 27, 28, 36, 39, 44, 47] : [10, 13, 14, 15, 16, 18, 19, 21, 23, 24, 29, 30, 32, 35, 37, 40, 41, 42, 45, 48]) as NonNullable<lorelei.Options["hair"]> } : {}) }).toDataUri();
    case "pixel-art": return createAvatar(pixelArt, { ...options, ...(gender ? { beardProbability: gender === "male" ? 25 : 0 } : {}), ...(gender && gender !== "neutral" ? { hair: namedHair(gender === "male" ? 24 : 21) as NonNullable<pixelArt.Options["hair"]> } : {}) }).toDataUri();
    default: return createAvatar(micah, { ...options, ...(gender ? { facialHairProbability: gender === "male" ? 25 : 0 } : {}), ...(gender && gender !== "neutral" ? { hair: (gender === "male" ? ["fonze", "mrT", "dougFunny", "mrClean", "dannyPhantom"] : ["full", "pixie"]) as NonNullable<micah.Options["hair"]> } : {}) }).toDataUri();
  }
}
export function avatarHash(value: string) {
  let hash = 2166136261;
  for (const character of value) hash = Math.imul(hash ^ character.codePointAt(0)!, 16777619);
  return hash >>> 0;
}
export function IdentityAvatar({ id, name, kind, value, className = "size-10" }: {
  id: string; name: string; kind: "user" | "agent"; value?: AvatarValue | null | undefined; className?: string;
}) {
  const [failed, setFailed] = useState("");
  const hash = avatarHash(`${kind}:${id}:${value?.seed ?? ""}`);
  const hue = hash % 360;
  const style = value?.style ?? (kind === "agent" ? "pixel" : "portrait");
  const illustration = useMemo(() => ["pixel", "initials", "emoji"].includes(style) || style === "image" && value?.image && failed !== value.image
    ? "" : illustrationSource(style, `${kind}:${id}:${value?.seed ?? ""}`, value ? value.gender : "neutral"), [style, kind, id, value?.seed, value?.gender, value?.image, failed]);
  const color = `hsl(${hue} 55% 42%)`;
  const background = `hsl(${hue} 55% 90%)`;
  const variedRobot = value?.gender !== undefined;
  return <span aria-hidden="true" className={`inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full ${className}`} style={{ backgroundColor: background, color }}>
    {style === "image" && value?.image && failed !== value.image ? <img alt="" src={value.image} className="size-full object-cover" onError={() => setFailed(value.image!)} />
      : style === "emoji" ? <span className="text-[1.35em] leading-none">{value?.emoji || "🙂"}</span>
      : style === "initials" ? <span className="font-semibold">{Array.from(name.trim()).slice(0, 2).join("").toLocaleUpperCase()}</span>
      : style === "pixel" || kind === "agent" && style === "image" ? <svg viewBox="0 0 64 64" className="size-full" shapeRendering="crispEdges">
        <path d={variedRobot ? ["M28 6h8v12h-8zM10 22h44v28H10zM4 30h6v12H4zm50 0h6v12h-6zM20 50h24v6H20z", "M24 8h4v12h-4zM36 8h4v12h-4zM12 20h40v32H12zM6 28h6v16H6zm46 0h6v16h-6zM24 52h16v6H24z", "M30 8h4v6h10v4H30zM8 24h48v24H8zM4 32h4v8H4zm52 0h4v8h-4zM16 48h8v8h-8zm24 0h8v8h-8z"][(hash >>> 4) % 3] : "M28 8h8v8h-8zM12 22h40v28H12zM6 30h6v12H6zm46 0h6v12h-6zM20 50h24v6H20z"} fill={color} />
        <path d="M18 28h28v16H18z" fill={background} />
        <path d={variedRobot ? ["M21 31h6v6h-6zm16 0h6v6h-6zM26 41h12v3H26z", "M20 32h9v3h-9zm15 0h9v3h-9zM28 39h8v6h-8z", "M21 30h3v3h3v3h-6zm16 0h3v3h3v3h-6zM25 40h4v3h6v-3h4v6H25z", "M20 31h8v6h-8zm16 0h8v6h-8zM24 40h16v2H24zm5 2h6v3h-6z"][(hash >>> 9) % 4] : hash % 2 ? "M22 31h6v6h-6zm14 0h6v6h-6zM26 40h12v3H26z" : "M21 33h8v3h-8zm14 0h8v3h-8zM29 39h6v4h-6z"} fill={color} />
      </svg> : <img alt="" src={illustration} className="size-full object-cover" />}
  </span>;
}

const emojis = ["🙂", "😊", "😎", "🤓", "🧑‍💻", "👩‍🚀", "🦊", "🐼", "🐱", "🐶", "🦁", "🦉", "🤖", "👾", "🚀", "💡", "✨", "🌻", "🌊", "🌙", "🎯", "🎨", "🧠", "⚡"];
export function AvatarEditor({ id, name, kind, value, onChange, readOnly = false, onBusyChange }: {
  id: string; name: string; kind: "user" | "agent"; value?: AvatarValue | null | undefined; onChange: (value: AvatarValue) => void; readOnly?: boolean;
  onBusyChange?: ((busy: boolean) => void) | undefined;
}) {
  useCommonTranslation();
  const input = useRef<HTMLInputElement>(null);
  const [error, setError] = useState("");
  const [uploading, setUploading] = useState(false);
  useEffect(() => { onBusyChange?.(uploading); }, [uploading, onBusyChange]);
  async function upload(file?: File) {
    if (!file) return;
    setError(""); setUploading(true);
    let bitmap: ImageBitmap | undefined;
    try {
      if (file.size > 5 * 1024 * 1024 || !["image/png", "image/jpeg", "image/webp"].includes(file.type)) throw Error(t("imageHelp"));
      bitmap = await createImageBitmap(file);
      if (!bitmap.width || !bitmap.height || bitmap.width > 8192 || bitmap.height > 8192) throw Error(t("imageHelp"));
      const canvas = document.createElement("canvas"); canvas.width = canvas.height = 128;
      const context = canvas.getContext("2d"); if (!context) throw Error(t("imageFailed"));
      const size = Math.min(bitmap.width, bitmap.height);
      context.drawImage(bitmap, (bitmap.width - size) / 2, (bitmap.height - size) / 2, size, size, 0, 0, 128, 128);
      const image = canvas.toDataURL("image/png");
      if (image.length > 96 * 1024) throw Error(t("imageHelp"));
      onChange({ style: "image", image });
    } catch (cause) { setError(cause instanceof Error ? cause.message : t("imageFailed")); }
    finally { bitmap?.close(); setUploading(false); if (input.current) input.current.value = ""; }
  }
  return <div className="space-y-3" data-avatar-editor>
    <div className="flex flex-col items-center gap-3 sm:flex-row sm:gap-4">
      <IdentityAvatar id={id} name={name} kind={kind} value={value} className="size-20 text-2xl" />
      {!readOnly && <div className="grid w-full min-w-0 flex-1 grid-cols-2 gap-2 sm:grid-cols-4" role="group" aria-label={t("avatar")}>
        {avatarStyles.map(style => <Button key={style} type="button" variant={style === (value?.style ?? (kind === "agent" ? "pixel" : "portrait")) ? "secondary" : "outline"} className="h-auto min-h-24 min-w-0 flex-col gap-2 px-2 py-3 whitespace-normal text-center" aria-label={t(style)} aria-pressed={value?.style === style || !value && style === (kind === "agent" ? "pixel" : "portrait")} onClick={() => onChange({ style, seed: value?.seed, gender: value?.gender ?? "neutral", ...(style === "emoji" ? { emoji: value?.emoji || emojis[kind === "agent" ? 12 : 0] } : {}) })}>
          <IdentityAvatar id={id} name={name} kind={kind} value={{ style, seed: value?.seed, gender: value?.gender ?? "neutral", ...(style === "emoji" ? { emoji: value?.emoji || emojis[kind === "agent" ? 12 : 0] } : {}) }} className="size-12 text-base" />
          <span className="max-w-full break-words text-xs">{t(style)}</span>
        </Button>)}
      </div>}
    </div>
    {!readOnly && <>
      {value?.style === "emoji" && <div className="grid grid-cols-6 gap-1 sm:grid-cols-8" role="group" aria-label={t("emoji")}>
        {emojis.map(emoji => <Button type="button" key={emoji} variant={value.emoji === emoji ? "secondary" : "ghost"} className="min-h-11 min-w-11 text-2xl" aria-label={emoji} aria-pressed={value.emoji === emoji} onClick={() => onChange({ style: "emoji", emoji })}>{emoji}</Button>)}
      </div>}
      {!readOnly && ["portrait", "adventurer", "notionists", "lorelei", "pixel-art"].includes(value?.style ?? (kind === "agent" ? "pixel" : "portrait")) && <AutoField label={t("gender")} help={t("genderHelp")}><select className="h-11 w-full min-w-0 rounded-md border bg-background px-3 text-sm" value={value?.gender ?? "neutral"}
        onChange={event => onChange({ style: value?.style ?? "portrait", gender: event.target.value as AvatarValue["gender"], seed: crypto.randomUUID() })}>
        {["neutral", "male", "female"].map(gender => <option key={gender} value={gender}>{t(gender)}</option>)}
      </select></AutoField>}
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" className="min-h-11" disabled={uploading || value?.style === "image" || value?.style === "emoji" || value?.style === "initials"} onClick={() => onChange({ style: value?.style ?? (kind === "agent" ? "pixel" : "portrait"), gender: value?.gender ?? "neutral", seed: crypto.randomUUID() })}>{t("regenerate")}</Button>
        <Button type="button" variant="outline" className="min-h-11" disabled={uploading} onClick={() => input.current?.click()}>{t(uploading ? "processing" : "upload")}</Button>
        <input ref={input} className="hidden" type="file" accept="image/png,image/jpeg,image/webp" onChange={event => void upload(event.target.files?.[0])} />
      </div>
      <p className="text-xs text-muted-foreground">{t("imageHelp")}</p>
      <p className="text-xs text-muted-foreground">{t("artworkCredits")} {[["Micah Lanier / Micah", "micah"], ["Lisa Wischofsky / Adventurer", "adventurer"], ["Zoish / Notionists", "notionists"], ["Lisa Wischofsky / Lorelei", "lorelei"], ["DiceBear / Pixel Art", "pixel-art"]].map(([name, slug], index) => <span key={slug}>{index > 0 && " · "}<a className="underline underline-offset-2" href={`https://www.dicebear.com/styles/${slug}/`} target="_blank" rel="noreferrer">{name}</a></span>)}</p>
    </>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </div>;
}
