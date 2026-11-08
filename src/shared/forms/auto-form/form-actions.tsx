import { createContext, type ComponentProps } from "react";
import { cn } from "cn";

/** Portaled editor panels save through the owning form and its existing validation. */
export const EditorFormActionsContext = createContext<{save:()=>void;saveAndWait?:()=>Promise<Record<string,unknown>|undefined>;saving:boolean;blocked:boolean}|null>(null);

/** Shared sticky footer actions fill the row; editor toolbars retain their layout. */
export function FormActions({ className, placement = "footer", ...props }: ComponentProps<"div"> & { placement?: "footer" | "toolbar" }) {
  return <div data-form-actions data-form-actions-placement={placement} className={cn("flex flex-wrap items-center gap-2", placement === "footer" && "sticky bottom-0 z-30 mt-4 w-full items-stretch gap-3 border-t bg-background/90 px-3 py-3 backdrop-blur supports-[backdrop-filter]:bg-background/75", className)} {...props} />;
}
