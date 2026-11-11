import { useEffect, useRef, useState } from "react";
import { commonText as text, useCommonTranslation } from "../i18n/i18n";
import { Button } from "./button";
import type { AvatarValue } from "../api/api";
export type { AvatarValue } from "../api/api";
const t = (key: string) => text(`identity:${key}`);
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
  const color = `hsl(${hue} 55% 42%)`;
  const background = `hsl(${hue} 55% 90%)`;
  return <span aria-hidden="true" className={`inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full ${className}`} style={{ backgroundColor: background, color }}>
    {style === "image" && value?.image && failed !== value.image ? <img alt="" src={value.image} className="size-full object-cover" onError={() => setFailed(value.image!)} />
      : style === "emoji" ? <span className="text-[1.35em] leading-none">{value?.emoji || "🙂"}</span>
      : style === "initials" ? <span className="font-semibold">{Array.from(name.trim()).slice(0, 2).join("").toLocaleUpperCase()}</span>
      : style === "pixel" || kind === "agent" && style === "image" ? <svg viewBox="0 0 64 64" className="size-full" shapeRendering="crispEdges">
        <path d="M28 8h8v8h-8zM12 22h40v28H12zM6 30h6v12H6zm46 0h6v12h-6zM20 50h24v6H20z" fill={color} />
        <path d="M18 28h28v16H18z" fill={background} />
        <path d={hash % 2 ? "M22 31h6v6h-6zm14 0h6v6h-6zM26 40h12v3H26z" : "M21 33h8v3h-8zm14 0h8v3h-8zM29 39h6v4h-6z"} fill={color} />
      </svg> : <svg viewBox="0 0 64 64" className="size-full">
        <path d="M8 64c0-16 10-24 24-24s24 8 24 24" fill={color} />
        <path d="M25 37h14v11c-4 5-10 5-14 0z" fill={hash % 2 ? "#b77c53" : "#e2ac80"} />
        <ellipse cx="32" cy="28" rx="16" ry="19" fill={hash % 2 ? "#c28b63" : "#f0bd94"} />
        <path d={hash % 3 ? "M15 28C10 4 51 0 50 30L41 16C34 24 22 16 15 28" : "M14 35C9 1 56 0 50 40l-6-20-14-6-10 12v14z"} fill={`hsl(${hash % 45} 30% 22%)`} />
        <path d="M25 28h2m10 0h2" stroke="#342a2a" strokeWidth="2.8" strokeLinecap="round" />
        <path d="M28 37q4 3 8 0" stroke="#825647" strokeWidth="1.6" fill="none" strokeLinecap="round" />
      </svg>}
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
      {!readOnly && <div className="grid w-full min-w-0 flex-1 grid-cols-2 gap-2 sm:flex sm:w-auto sm:flex-wrap">
        {(["portrait", "pixel", "initials", "emoji"] as const).map(style => <Button key={style} type="button" variant={style === (value?.style ?? (kind === "agent" ? "pixel" : "portrait")) ? "secondary" : "outline"} className="min-h-11" aria-pressed={value?.style === style || !value && style === (kind === "agent" ? "pixel" : "portrait")} onClick={() => onChange({ style, seed: value?.seed, ...(style === "emoji" ? { emoji: value?.emoji || emojis[kind === "agent" ? 12 : 0] } : {}) })}>{t(style)}</Button>)}
      </div>}
    </div>
    {!readOnly && <>
      {value?.style === "emoji" && <div className="grid grid-cols-6 gap-1 sm:grid-cols-8" role="group" aria-label={t("emoji")}>
        {emojis.map(emoji => <Button type="button" key={emoji} variant={value.emoji === emoji ? "secondary" : "ghost"} className="min-h-11 min-w-11 text-2xl" aria-label={emoji} aria-pressed={value.emoji === emoji} onClick={() => onChange({ style: "emoji", emoji })}>{emoji}</Button>)}
      </div>}
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" className="min-h-11" disabled={uploading || value?.style === "image" || value?.style === "emoji"} onClick={() => onChange({ style: value?.style ?? (kind === "agent" ? "pixel" : "portrait"), seed: crypto.randomUUID() })}>{t("regenerate")}</Button>
        <Button type="button" variant="outline" className="min-h-11" disabled={uploading} onClick={() => input.current?.click()}>{t(uploading ? "processing" : "upload")}</Button>
        <input ref={input} className="hidden" type="file" accept="image/png,image/jpeg,image/webp" onChange={event => void upload(event.target.files?.[0])} />
      </div>
      <p className="text-xs text-muted-foreground">{t("imageHelp")}</p>
    </>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </div>;
}
