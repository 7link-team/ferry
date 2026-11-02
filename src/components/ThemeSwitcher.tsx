import { useState } from "react";
import { useTranslation } from "react-i18next";
import "../i18n";
import { Dialog } from "radix-ui";
import { Moon, Palette, Sun, X } from "lucide-react";
import { Button } from "./ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./ui/dropdown-menu";
import { PALETTES, useTheme, type Mode } from "../theme";
import { CustomThemePanel } from "./CustomThemePanel";

export function ThemeSwitcher() {
  const { t } = useTranslation("theme", { keyPrefix: "" });
  const label = (key: string) => t(key, { keySeparator: false });
  const theme = useTheme();
  const [editing, setEditing] = useState(false);
  return <>
    <DropdownMenu>
      <DropdownMenuTrigger asChild><Button variant="ghost" size="sm" aria-label={label("label")}><Palette />{label(theme.palette)}</Button></DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {PALETTES.map(palette => <DropdownMenuItem key={palette.value} onClick={() => { theme.set({ palette: palette.value }); if (palette.value === "custom") setEditing(true); }}>{label(palette.value)}{palette.value === theme.palette && " ✓"}</DropdownMenuItem>)}
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
