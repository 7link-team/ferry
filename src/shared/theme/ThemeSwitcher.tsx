import { Moon,Palette,Sun,X } from "lucide-react";
import { Dialog } from "radix-ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import "../i18n/i18n";
import { Button } from "../ui/button";
import { DropdownMenu,DropdownMenuContent,DropdownMenuItem,DropdownMenuTrigger } from "../ui/dropdown-menu";
import { CustomThemePanel } from "./CustomThemePanel";
import { PALETTES,useTheme,type Mode } from "./theme";

// Same palette previews as the reference theme picker.
const previews = {
  modern: ["#fafafa", "#171717"], business: ["#fafaf9", "#6366f1"],
  eyeCare: ["#f5f0e6", "#5a7c5a"], pinkKawaii: ["#fff0f6", "#e8388a"],
  neoBrutalism: ["#fef08a", "#ff006e"], custom: ["#fafafa", "#6366f1"],
} as const;

export function ThemeSwitcher() {
  const { t } = useTranslation("theme", { keyPrefix: "" });
  const label = (key: string) => t(key, { keySeparator: false });
  const theme = useTheme();
  const [editing, setEditing] = useState(false);
  return <>
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button variant="ghost" size="sm" aria-label={label("label")}><Palette />{label(theme.palette)}</Button></DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-56">
        {PALETTES.map(palette => <DropdownMenuItem key={palette.value} onClick={() => { theme.set({ palette: palette.value }); if (palette.value === "custom") setEditing(true); }}>
          <span aria-hidden="true" className="flex size-6 shrink-0 items-center justify-center border" style={{ background: previews[palette.value][0], borderRadius: palette.value === "neoBrutalism" ? 0 : 5 }}><span className="size-2.5" style={{ background: previews[palette.value][1], borderRadius: palette.value === "neoBrutalism" ? 0 : 3 }} /></span>
          <span className="flex-1">{label(palette.value)}</span>{palette.value === theme.palette && " ✓"}
        </DropdownMenuItem>)}
        <div className="my-1 border-t" />
        {(["light", "dark", "system"] as Mode[]).map(mode => <DropdownMenuItem key={mode} onClick={() => theme.set({ mode })}>{label(mode === "system" ? "followSystem" : `mode.${mode}`)}{mode === theme.mode && " ✓"}</DropdownMenuItem>)}
      </DropdownMenuContent>
    </DropdownMenu>
    <Button variant="ghost" size="icon" aria-label={label("toggleMode")} onClick={() => theme.set({ mode: theme.resolved === "dark" ? "light" : "dark" })}>{theme.resolved === "dark" ? <Moon /> : <Sun />}</Button>
    <Dialog.Root open={editing} onOpenChange={setEditing}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[calc(100%-2rem)] max-w-3xl -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-xl border bg-background p-5 text-foreground shadow-lg">
          <Dialog.Title className="pr-10 text-lg font-semibold">{label("customize")}</Dialog.Title>
          <Dialog.Description className="mb-5 mt-1 text-sm text-muted-foreground">{label("description")}</Dialog.Description>
          <Dialog.Close asChild><Button variant="ghost" size="icon" aria-label={label("close")} className="absolute right-3 top-3"><X /></Button></Dialog.Close>
          <CustomThemePanel />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  </>;
}
