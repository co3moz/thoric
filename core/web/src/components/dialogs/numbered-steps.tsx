import * as React from "react"
import { cn } from "cn"

import { t } from "@/lib/i18n"

/**
 * Steps that run in order, such as the parts of a merged build or the steps of a compose: each
 * numbered, joined to the next by a line, with a heading row and what belongs to it below.
 */
export function NumberedSteps({ children }: { children: React.ReactNode }) {
  return <ol className="flex flex-col">{children}</ol>
}

export function NumberedStep({
  number,
  last,
  heading,
  children,
}: {
  /** Its place in the order, from 1. */
  number: number
  last: boolean
  /** The heading row: the step's name first, then what is known of it. */
  heading: React.ReactNode
  children?: React.ReactNode
}) {
  return (
    <li className="grid grid-cols-[1.5rem_minmax(0,1fr)] gap-x-3">
      <div className="flex flex-col items-center">
        <span
          aria-hidden
          className="flex size-6 shrink-0 items-center justify-center rounded-full bg-input/70 text-xs font-medium text-muted-foreground tabular-nums dark:bg-muted"
        >
          {number}
        </span>
        {!last && <span aria-hidden className="mt-1 w-px flex-1 bg-border" />}
      </div>
      <div className={cn("flex min-w-0 flex-col gap-3", !last && "pb-6")}>
        <div className="flex min-h-6 flex-wrap items-center gap-x-2 gap-y-1">
          <span className="sr-only">{t("setup.stepNumber", { number })}</span>
          {heading}
        </div>
        {children}
      </div>
    </li>
  )
}
