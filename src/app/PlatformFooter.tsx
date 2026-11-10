import { LoadingSkeleton } from "../shared/ui/loading-skeleton";
import {lazy, Suspense, useEffect, useMemo, useState, type ReactNode} from "react";
import {api} from "../shared/api/api";
import {commonText as t, useCommonTranslation} from "../shared/i18n/i18n";
import {AutoField} from "../shared/forms/auto-form";
import {sanitizeRichText} from "../features/maintenance/value-content";
import {Tabs, TabsList, TabsTrigger} from "../shared/ui/tabs";
import {typedText} from "../shared/i18n/i18n";
import {defaultSiteSettings, SiteSettingsContext, type SiteSettings} from "../shared/site/settings";

const RichTextEditor = lazy(() => import("../features/maintenance/RichTextValueEditor"));
export const FOOTER_UPDATED = "platform:footer-updated";

export function FooterContent({value}: {value: string}) {
  const content = useMemo(() => sanitizeRichText(value), [value]);
  if (!content.replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").trim() && !/<img\b/i.test(content)) return null;
  return <div className="min-w-0 whitespace-pre-line break-words text-center text-sm text-muted-foreground [overflow-wrap:anywhere] [&_a]:underline [&_a]:underline-offset-2 [&_img]:inline-block [&_img]:max-w-full [&_table]:max-w-full" dangerouslySetInnerHTML={{__html: content}} />;
}

/** A normal layout row: footer content never covers controls or the canvas. */
export function PlatformFrame({children}: {children: ReactNode}) {
  const [content, setContent] = useState("");
  const [site, setSite] = useState(defaultSiteSettings);
  useEffect(() => {
    let active = true;
    let generation = 0;
    const read = () => {
      const current = ++generation;
      void api<{content: string} & Partial<SiteSettings>>("/platform/footer", {cache:"no-store"}).then(row => {
        if (active && current === generation) {
          setContent(row.content);
          setSite({site_name:row.site_name ?? defaultSiteSettings.site_name,
            site_title_prefix_enabled:row.site_title_prefix_enabled ?? defaultSiteSettings.site_title_prefix_enabled,
            site_title_visible_prefix:row.site_title_visible_prefix ?? defaultSiteSettings.site_title_visible_prefix,
            site_title_hidden_prefix:row.site_title_hidden_prefix ?? defaultSiteSettings.site_title_hidden_prefix});
        }
      }).catch(() => { /* Keep the last known footer if the service is temporarily unavailable. */ });
    };
    read();
    window.addEventListener(FOOTER_UPDATED, read);
    return () => { active = false; window.removeEventListener(FOOTER_UPDATED, read); };
  }, []);
  const clean = useMemo(() => sanitizeRichText(content), [content]);
  const visible = clean.replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").trim() || /<img\b/i.test(clean);
  return <SiteSettingsContext.Provider value={site}><div className="flex h-full min-h-0 min-w-0 flex-col">
    <div data-platform-content className="min-h-0 min-w-0 flex-1 overflow-auto">{children}</div>
    {visible && <footer data-platform-footer className="max-h-[25vh] shrink-0 overflow-auto border-t bg-background px-4 py-2"><FooterContent value={clean} /></footer>}
  </div></SiteSettingsContext.Provider>;
}

export function PlatformFooterEditor({value, readOnly, onChange}: {value:string; readOnly:boolean; onChange:(value:string)=>void}) {
  useCommonTranslation();
  const [advanced, setAdvanced] = useState(false);
  return <AutoField label={t("resource-domain:platform.footerContent")} help={t("resource-domain:platform.footerHelp")}><div className="min-w-0 space-y-3">
    <Tabs value={advanced?"html":"rich"} onValueChange={value=>setAdvanced(value==="html")}><TabsList><TabsTrigger value="rich">{typedText("type.richtext")}</TabsTrigger><TabsTrigger value="html">{t("resource-domain:platform.footerHtml")}</TabsTrigger></TabsList></Tabs>
    {advanced ? <AutoField label={t("resource-domain:platform.footerHtml")}><textarea aria-label={t("resource-domain:platform.footerHtml")} className="min-h-36 w-full rounded-md border bg-background px-3 py-2 font-mono text-sm" maxLength={65536} value={value} disabled={readOnly} onChange={event=>onChange(event.target.value)} /></AutoField>
      : <Suspense fallback={<LoadingSkeleton variant="editor" />}><RichTextEditor value={value} readOnly={readOnly} onChange={onChange} /></Suspense>}
    <div className="border-t pt-3" data-footer-preview><FooterContent value={value} /></div>
  </div></AutoField>;
}
