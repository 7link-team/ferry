import type { MouseEventHandler } from "react";
import { Ellipsis, GitBranch, House, Layers, MessagesSquare, Server, Settings, Users, Workflow, Activity } from "lucide-react";
import { useTranslation } from "react-i18next";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../shared/ui/dropdown-menu";
import { cn } from "cn";

export interface MobileDestination {
  key: string;
  label: string;
  href: string;
  onClick: MouseEventHandler<HTMLAnchorElement>;
}
const icons = { dashboard: House, conversations: MessagesSquare, workflow: Workflow, host: Server, deploy: Layers, repository: GitBranch, ops: Settings, workspaces: Layers, users: Users, monitor: Activity, settings: Settings };

/** Uses the same authorized destinations and URLs as desktop navigation. */
export function MobileNavigation({ items, active, primary = ["dashboard", "conversations", "workflow", "host"] }: { items: MobileDestination[]; active: string; primary?: string[] }) {
  const { t } = useTranslation();
  const pinned = primary.map(key => items.find(item => item.key === key)).filter((item): item is MobileDestination => !!item).slice(0, 4);
  const overflow = items.filter(item => !pinned.includes(item));
  return <nav aria-label={t("app.mobileNavigation")} className="z-30 flex min-w-0 shrink-0 items-stretch border-t bg-background px-1 pt-1 pb-[max(.25rem,env(safe-area-inset-bottom))] md:hidden">
    {pinned.map(item => {
      const Icon = icons[item.key as keyof typeof icons] ?? Layers;
      return <a key={item.key} href={item.href} onClick={item.onClick} aria-current={active === item.key ? "page" : undefined} className={cn("flex min-h-14 min-w-0 flex-1 flex-col items-center justify-center gap-1 rounded-lg px-1 py-1 text-center text-[11px] leading-tight break-words focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring", active === item.key ? "bg-accent font-semibold text-accent-foreground" : "text-foreground")}><Icon className="size-5 shrink-0" aria-hidden="true"/><span>{item.label}</span></a>;
    })}
    {!!overflow.length && <DropdownMenu key={active}><DropdownMenuTrigger asChild><button type="button" className={cn("flex min-h-14 min-w-0 flex-1 flex-col items-center justify-center gap-1 rounded-lg px-1 py-1 text-[11px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring", overflow.some(item => item.key === active) && "bg-accent font-semibold text-accent-foreground")}><Ellipsis className="size-5" aria-hidden="true"/><span>{t("app.more")}</span></button></DropdownMenuTrigger><DropdownMenuContent side="top" align="end" collisionPadding={8} className="max-h-[60dvh] w-60 max-w-[calc(100vw-1rem)] overflow-y-auto">{overflow.map(item => <DropdownMenuItem key={item.key} asChild><a href={item.href} onClick={item.onClick} aria-current={active === item.key ? "page" : undefined} className="min-h-11 whitespace-normal break-words">{item.label}</a></DropdownMenuItem>)}</DropdownMenuContent></DropdownMenu>}
  </nav>;
}
