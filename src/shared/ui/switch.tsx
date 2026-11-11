"use client"

import { cn } from "cn"
import { Switch as SwitchPrimitive } from "radix-ui"
import type * as React from "react"

function Switch({ className, ...props }: React.ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn("group/toggle peer inline-flex h-11 w-12 shrink-0 cursor-pointer items-center justify-center rounded-md outline-none transition-shadow focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:ring-[3px] aria-invalid:ring-destructive/20", className)}
      {...props}
    >
      <span aria-hidden="true" className="inline-flex h-6 w-11 items-center rounded-full border-2 border-transparent bg-muted-foreground/60 transition-colors group-data-[state=checked]/toggle:bg-primary motion-reduce:transition-none">
        <SwitchPrimitive.Thumb data-slot="switch-thumb" className="pointer-events-none block size-5 translate-x-0 rounded-full bg-background shadow-xs transition-transform group-data-[state=checked]/toggle:translate-x-5 motion-reduce:transition-none" />
      </span>
    </SwitchPrimitive.Root>
  )
}

export { Switch }
