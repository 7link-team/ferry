import { Check, Globe } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import "./i18n";
import {
  currentLocale,
  localeNames,
  setLocale,
  supportedLocales,
  type Locale,
} from "./locale";

export function LanguageSwitcher({ floating = false, showLabel = false }: { floating?: boolean; showLabel?: boolean }) {
  const { t } = useTranslation();
  const locale = currentLocale();
  return (
    <div
      className={
        floating ? "fixed end-3 top-2 z-40 rounded-md bg-card" : "shrink-0"
      }
    >
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            aria-label={t("language.label")}
            aria-description={localeNames[locale]}
            title={`${t("language.label")}: ${localeNames[locale]}`}
            className={`h-11 gap-1.5 rounded-md px-2 has-[>svg]:px-2 text-foreground transition-colors hover:bg-accent hover:text-accent-foreground ${showLabel ? "max-w-full" : "max-sm:size-11 max-sm:p-0 max-sm:has-[>svg]:px-0"}`}
          >
            <Globe aria-hidden="true" className="size-4 shrink-0" />
            <span className={`${showLabel ? "inline" : "hidden sm:inline"} max-w-[100px] truncate text-sm font-medium`} lang={locale}>
              {localeNames[locale]}
            </span>
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          collisionPadding={8}
          sideOffset={8}
          className="language-menu max-h-96 w-60 p-2"
        >
          <div className="mb-1 px-3 py-2 text-xs font-semibold text-popover-foreground">
            {t("language.label")}
          </div>
          <DropdownMenuRadioGroup
            value={locale}
            onValueChange={(value) => setLocale(value as Locale)}
            aria-label={t("language.label")}
            className="space-y-1"
          >
            {supportedLocales.map((value) => (
              <DropdownMenuRadioItem
                key={value}
                value={value}
                lang={value}
                textValue={localeNames[value]}
                className="min-h-11 w-full gap-3 rounded-md border border-transparent px-3 py-2 text-popover-foreground transition-colors data-[state=checked]:border-primary/25 data-[state=checked]:bg-primary/10 data-[state=checked]:font-semibold data-[state=checked]:text-popover-foreground data-[state=checked]:focus:bg-primary/15 [&>span:first-child]:hidden"
              >
                <span aria-hidden="true" className="inline-flex w-8 shrink-0 items-center justify-center rounded border border-current/20 text-[10px] font-semibold leading-6">
                  {value.split("-")[0]!.toUpperCase()}
                </span>
                <span className="min-w-0 flex-1 truncate text-left">{localeNames[value]}</span>
                {value === locale && <Check aria-hidden="true" className="size-3.5 shrink-0" />}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
