import { createContext, type ComponentProps } from "react";
import { cn } from "cn";

/** Portaled editor panels save through the owning form and its existing validation. */
export const EditorFormActionsContext = createContext<{save:()=>void;saveAndWait?:()=>Promise<Record<string,unknown>|undefined>;saving:boolean;blocked:boolean}|null>(null);

/** Reference CrudEditButton's sticky toolbar, shared by detail and edit modes. */
export function FormActions({ className, placement = "footer", ...props }: ComponentProps<"div"> & { placement?: "footer" | "toolbar" }) {
  return <div data-form-actions className={cn("flex flex-wrap items-center gap-2", placement === "footer" && "sticky bottom-0 z-30 mt-6 border-t bg-background/90 py-3 backdrop-blur supports-[backdrop-filter]:bg-background/75", className)} {...props} />;
}
