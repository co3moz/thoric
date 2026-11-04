import { RiCheckLine } from "@remixicon/react"
import { cn } from "cn"

import { Badge } from "@/components/ui/badge"
import { FieldDescription } from "@/components/ui/field"
import { composeEntries, composeSteps, missingValues, stepTitle } from "@/lib/compose"
import { plural, t } from "@/lib/i18n"
import type { ImageItem } from "@/lib/types"
import { ParameterField } from "./parameter-field"

type Compose = NonNullable<ImageItem["compose"]>

/**
 * The values an image's compose asks for, filled in with its defaults. One step shows its fields
 * as they are; several, such as templates merged into one image, are numbered in the order they
 * run, each with the values it asks for and whether one is still missing, since two steps may ask
 * for the same name. Every step stays open, so no required value is hidden. `inherited` names the
 * steps the image keeps from the saved image it was built on.
 */
export function SetupFields({
  compose,
  values,
  onChange,
  inherited,
}: {
  compose: Compose
  values: Record<string, string>
  onChange: (values: Record<string, string>) => void
  inherited?: { name: string; ids: Set<string> } | null
}) {
  const steps = composeSteps(compose)
  const entries = composeEntries(compose)
  const missing = missingValues(compose, values)
  const fields = (step: (typeof steps)[number]) => (
    <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
      {entries
        .filter((entry) => entry.step === step)
        .map(({ parameter, key }) => (
          <ParameterField
            key={`${key}-${parameter.type}`}
            parameter={parameter}
            value={values[key] ?? parameter.default}
            onChange={(value) => onChange({ ...values, [key]: value })}
            compact
            createVolumes
          />
        ))}
    </div>
  )

  if (steps.length === 1)
    return entries.length ? (
      <>
        <FieldDescription className="text-xs">{t("compose.valuesHint")}</FieldDescription>
        {fields(steps[0])}
      </>
    ) : (
      <FieldDescription className="text-xs">{t("compose.runsScript")}</FieldDescription>
    )

  return (
    <>
      <FieldDescription className="text-xs">{t("compose.valuesHintSteps")}</FieldDescription>
      <ol className="flex flex-col">
        {steps.map((step, index) => {
          const last = index === steps.length - 1
          const asks = step.parameters.length
          const from = inherited && step.id && inherited.ids.has(step.id) ? inherited.name : ""
          return (
            <li key={step.id || index} className="grid grid-cols-[1.5rem_minmax(0,1fr)] gap-x-3">
              <div className="flex flex-col items-center">
                <span
                  aria-hidden
                  className="flex size-6 shrink-0 items-center justify-center rounded-full bg-input/70 text-xs font-medium text-muted-foreground tabular-nums dark:bg-muted"
                >
                  {index + 1}
                </span>
                {!last && <span aria-hidden className="mt-1 w-px flex-1 bg-border" />}
              </div>
              <div className={cn("flex min-w-0 flex-col gap-3", !last && "pb-6")}>
                <div className="flex min-h-6 flex-wrap items-center gap-x-2 gap-y-1">
                  <span className="font-medium">
                    <span className="sr-only">{t("setup.stepNumber", { number: index + 1 })}</span>
                    {stepTitle(step)}
                  </span>
                  {asks > 0 && (
                    <span className="text-xs text-muted-foreground">
                      {plural(asks, {
                        one: "compose.parameterCount.one",
                        other: "compose.parameterCount.other",
                      })}
                    </span>
                  )}
                  {from && <Badge variant="outline">{t("setup.fromBase", { name: from })}</Badge>}
                  {step.root && (
                    <Badge variant="outline" className="text-warning">
                      {t("setup.root")}
                    </Badge>
                  )}
                  <span className="ml-auto flex items-center gap-1 text-xs">
                    {missing[index] > 0 ? (
                      <span
                        aria-label={plural(missing[index], {
                          one: "sections.missing.one",
                          other: "sections.missing.other",
                        })}
                        className="flex h-4.5 min-w-4.5 items-center justify-center rounded-full bg-destructive px-1 text-[11px] leading-none font-medium text-background tabular-nums"
                      >
                        {missing[index]}
                      </span>
                    ) : (
                      asks > 0 && (
                        <span className="flex items-center gap-1 text-success">
                          <RiCheckLine aria-hidden className="size-3.5" />
                          {t("setup.ready")}
                        </span>
                      )
                    )}
                  </span>
                </div>
                {asks > 0 ? (
                  fields(step)
                ) : (
                  <FieldDescription className="-mt-2 text-xs">
                    {t("compose.stepRunsScript")}
                  </FieldDescription>
                )}
              </div>
            </li>
          )
        })}
      </ol>
    </>
  )
}
