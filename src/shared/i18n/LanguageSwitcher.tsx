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

export function LanguageSwitcher({ floating = false }: { floating?: boolean }) {
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
            className="h-8 gap-1.5 rounded-md px-2 has-[>svg]:px-2 text-foreground transition-colors hover:bg-accent hover:text-accent-foreground max-sm:size-8 max-sm:p-0 max-sm:has-[>svg]:px-0"
          >
            <Globe aria-hidden="true" className="size-4 shrink-0" />
            <span className="hidden max-w-[100px] truncate text-sm font-medium sm:inline" lang={locale}>
              {localeNames[locale]}
            </span>
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          collisionPadding={8}
          sideOffset={8}
          className="max-h-80 w-56 p-0"
        >
          <div className="border-b border-border px-3 py-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {t("language.label")}
          </div>
          <DropdownMenuRadioGroup
            value={locale}
            onValueChange={(value) => setLocale(value as Locale)}
            aria-label={t("language.label")}
            className="p-1"
          >
            {supportedLocales.map((value) => (
              <DropdownMenuRadioItem
                key={value}
                value={value}
                lang={value}
                textValue={localeNames[value]}
                className="w-full gap-2 rounded-md px-2 py-1.5 text-popover-foreground transition-colors data-[state=checked]:bg-primary data-[state=checked]:font-medium data-[state=checked]:text-primary-foreground data-[state=checked]:focus:bg-primary data-[state=checked]:focus:text-primary-foreground [&>span:first-child]:hidden"
              >
                <span aria-hidden="true" className={`inline-flex w-7 shrink-0 items-center justify-center rounded text-[10px] font-semibold leading-5 ${value === locale ? "bg-primary-foreground/20 text-primary-foreground" : "bg-muted text-muted-foreground"}`}>
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
