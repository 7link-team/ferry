import { Check,Monitor,Moon,Palette,Sun,X } from "lucide-react";
import { Dialog } from "radix-ui";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import "../i18n/i18n";
import { Button } from "../ui/button";
import { DropdownMenu,DropdownMenuContent,DropdownMenuItem,DropdownMenuTrigger } from "../ui/dropdown-menu";
import { CustomThemePanel } from "./CustomThemePanel";
import { PALETTES,useTheme } from "./theme";

// Same palette previews as the reference theme picker.
const previews = {
  modern: ["#fafafa", "#171717"], business: ["#fafaf9", "#6366f1"],
  eyeCare: ["#f5f0e6", "#5a7c5a"], pinkKawaii: ["#fff0f6", "#e8388a"],
  // Official Raft layer.frame and attention.400 from raft/tokens.json.
  neoBrutalism: ["oklch(0.8829 0.1623 91.9)", "oklch(0.7999 0.1308 354.26)"], custom: ["#fafafa", "#6366f1"],
} as const;
const modeIcons = { system: Monitor, light: Sun, dark: Moon } as const;

export function ThemeSwitcher() {
  const { t } = useTranslation("theme", { keyPrefix: "" });
  const label = (key: string) => t(key, { keySeparator: false });
  const theme = useTheme();
  const [editing, setEditing] = useState(false);
  return <>
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button variant="ghost" size="sm" aria-label={label("label")} className="h-8 gap-1.5 rounded-md px-2 has-[>svg]:px-2 text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground max-sm:size-8 max-sm:p-0 max-sm:has-[>svg]:px-0"><Palette aria-hidden="true" className="size-4 shrink-0" /><span className="hidden max-w-[90px] truncate text-xs sm:inline">{label(theme.palette)}</span></Button></DropdownMenuTrigger>
      <DropdownMenuContent align="end" sideOffset={8} collisionPadding={8} className="w-64 p-1">
        <div className="mb-1 px-2 py-1 text-xs font-semibold uppercase tracking-wider text-muted-foreground">{label("appearance")}</div>
        <div className="mb-2 flex gap-1" role="group" aria-label={label("appearance")}>
          {(["system", "light", "dark"] as const).map(mode => {
            const Icon = modeIcons[mode];
            return <DropdownMenuItem key={mode} aria-label={label(mode === "system" ? "followSystem" : `mode.${mode}`)} onSelect={() => theme.set({ mode })} className={`flex flex-1 justify-center gap-1.5 rounded-lg px-2 py-1.5 text-xs transition-colors ${mode === theme.mode ? "bg-primary text-primary-foreground focus:bg-primary focus:text-primary-foreground" : "text-popover-foreground hover:bg-accent hover:text-accent-foreground"}`}>
              <Icon aria-hidden="true" className={`size-3.5 shrink-0 ${mode === theme.mode ? "text-primary-foreground" : "text-popover-foreground"}`} /><span>{label(`mode.${mode}`)}</span>
            </DropdownMenuItem>;
          })}
        </div>
        <div className="my-1 h-px bg-border" />
        <div className="mb-1 px-2 py-1 text-xs font-semibold uppercase tracking-wider text-muted-foreground">{label("label")}</div>
        {PALETTES.map(palette => <DropdownMenuItem key={palette.value} onSelect={() => { theme.set({ palette: palette.value }); if (palette.value === "custom") setEditing(true); }} className={`w-full gap-3 rounded-lg px-3 py-2 transition-colors ${palette.value === theme.palette ? "bg-primary font-medium text-primary-foreground focus:bg-primary focus:text-primary-foreground" : "text-popover-foreground hover:bg-accent hover:text-accent-foreground"}`}>
          <span aria-hidden="true" className="flex size-6 shrink-0 items-center justify-center border" style={{ background: previews[palette.value][0], borderColor: palette.value === theme.palette ? "rgba(255,255,255,0.3)" : "rgba(0,0,0,0.1)", borderRadius: palette.value === "neoBrutalism" ? 4 : 5 }}><span className="block size-2.5" style={{ background: previews[palette.value][1], borderRadius: palette.value === "neoBrutalism" ? 2 : 3 }} /></span>
          <span className="flex-1 truncate text-left">{label(palette.value)}</span>{palette.value === "custom" ? <Palette aria-hidden="true" className={`size-4 shrink-0 opacity-70 ${palette.value === theme.palette ? "text-primary-foreground" : "text-popover-foreground"}`} /> : palette.value === theme.palette && <Check aria-hidden="true" className="size-4 shrink-0 text-primary-foreground" />}
        </DropdownMenuItem>)}
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
