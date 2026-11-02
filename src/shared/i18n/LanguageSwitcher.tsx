import { ChevronDown, Languages } from "lucide-react";
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
            className="max-sm:size-8 max-sm:p-0"
          >
            <Languages aria-hidden="true" />
            <span className="hidden sm:inline" lang={locale}>
              {localeNames[locale]}
            </span>
            <ChevronDown
              aria-hidden="true"
              className="hidden size-3.5 text-muted-foreground sm:block"
            />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          collisionPadding={8}
          className="min-w-44"
        >
          <DropdownMenuRadioGroup
            value={locale}
            onValueChange={(value) => setLocale(value as Locale)}
            aria-label={t("language.label")}
          >
            {supportedLocales.map((value) => (
              <DropdownMenuRadioItem
                key={value}
                value={value}
                lang={value}
                textValue={localeNames[value]}
                className="data-[state=checked]:bg-accent data-[state=checked]:font-medium data-[state=checked]:text-accent-foreground"
              >
                {localeNames[value]}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
