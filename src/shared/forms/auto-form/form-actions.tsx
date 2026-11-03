import type { ComponentProps } from "react";
import { cn } from "cn";

/** Reference CrudEditButton's sticky toolbar, shared by detail and edit modes. */
export function FormActions({ className, ...props }: ComponentProps<"div">) {
  return <div data-form-actions className={cn("sticky bottom-0 z-30 -mx-1 mt-6 flex flex-wrap items-center gap-2 border-t bg-background/90 px-1 py-3 backdrop-blur supports-[backdrop-filter]:bg-background/75", className)} {...props} />;
}
