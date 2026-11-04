import * as React from "react"
import { cn } from "cn"

import { Field, FieldContent, FieldDescription, FieldLabel } from "@/components/ui/field"
import { Switch } from "@/components/ui/switch"
import { imageSteps } from "@/lib/compose"
import { formatList, t } from "@/lib/i18n"
import type { ImageItem } from "@/lib/types"
import { tile } from "./form-sections"

/**
 * The compose of the saved image a build starts from, which the new image keeps before its own
 * unless it is turned off. What that image's own build installed, with the options chosen then,
 * is in its layers already; only what a new container asks for and sets up is carried over.
 */
export function BuildBase({
  base,
  checked,
  onCheckedChange,
}: {
  base: Pick<ImageItem, "name" | "compose">
  checked: boolean
  onCheckedChange: (checked: boolean) => void
}) {
  const id = React.useId()
  // One step goes by the image's name, which the label already says.
  const steps = imageSteps(base)
  return (
    <Field orientation="horizontal" className={cn("items-start gap-3 px-3 py-2.5", tile)}>
      <Switch id={id} className="mt-0.5" checked={checked} onCheckedChange={onCheckedChange} />
      <FieldContent className="gap-1">
        <FieldLabel htmlFor={id} className="leading-snug">
          {t("build.inherit", { name: base.name })}
        </FieldLabel>
        <FieldDescription className="text-xs">
          {checked
            ? t("build.inheritHint", { name: base.name })
            : t("build.inheritOff", { name: base.name })}
          {checked &&
            steps.length > 1 &&
            ` ${t("compose.baseSteps", { steps: formatList(steps) })}`}
        </FieldDescription>
      </FieldContent>
    </Field>
  )
}
