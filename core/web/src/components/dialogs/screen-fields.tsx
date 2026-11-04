import * as React from "react"

import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { t } from "@/lib/i18n"
import { RESOLUTIONS, type SpecDraft } from "./spec-fields"

/** The choice of a size of one's own, which no `WIDTHxHEIGHT` can be. */
const CUSTOM = "custom"
/** The sides the manager accepts (`ContainerSpec::validate`). */
const WIDTH = { min: 320, max: 7680 }
const HEIGHT = { min: 240, max: 4320 }

/** A choice of a few values as a row of pressed buttons, the style every section's choices share. */
export const choiceGroup = "flex-wrap justify-start"
export const choiceItem =
  "px-3 font-normal text-muted-foreground data-[state=on]:border-primary data-[state=on]:font-medium data-[state=on]:text-foreground"

/** The desktop's screen and how much of the host it may take. */
export function ScreenFields({
  draft,
  onChange,
}: {
  draft: SpecDraft
  onChange: (draft: SpecDraft) => void
}) {
  const id = React.useId()
  const set = (patch: Partial<SpecDraft>) => onChange({ ...draft, ...patch })
  // A size off the list is custom; a listed one only once Custom is picked, so typing a listed
  // size into the fields does not close them.
  const [customPicked, setCustomPicked] = React.useState(
    () => !RESOLUTIONS.includes(draft.resolution)
  )
  const custom = customPicked || !RESOLUTIONS.includes(draft.resolution)
  const [width = "", height = ""] = draft.resolution.split("x")

  return (
    <FieldGroup>
      <Field>
        <FieldLabel id={`${id}-resolution`}>{t("spec.resolution")}</FieldLabel>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-labelledby={`${id}-resolution`}
          value={custom ? CUSTOM : draft.resolution}
          onValueChange={(value) => {
            // Pressing the chosen size again keeps it.
            if (!value) return
            setCustomPicked(value === CUSTOM)
            if (value !== CUSTOM) set({ resolution: value })
          }}
          className={choiceGroup}
        >
          {RESOLUTIONS.map((value) => (
            <ToggleGroupItem key={value} value={value} className={choiceItem}>
              {value.replace("x", "×")}
            </ToggleGroupItem>
          ))}
          <ToggleGroupItem value={CUSTOM} className={choiceItem}>
            {t("spec.resolution.custom")}
          </ToggleGroupItem>
        </ToggleGroup>
        {custom && (
          <>
            <div className="flex items-center gap-2 sm:max-w-sm">
              <Input
                type="number"
                min={WIDTH.min}
                max={WIDTH.max}
                step={1}
                required
                aria-label={t("spec.resolution.width")}
                aria-describedby={`${id}-resolution-hint`}
                placeholder={t("spec.resolution.width")}
                value={width}
                onChange={(event) => set({ resolution: `${event.target.value}x${height}` })}
                className="min-w-0 flex-1"
              />
              <span aria-hidden className="text-muted-foreground">
                ×
              </span>
              <Input
                type="number"
                min={HEIGHT.min}
                max={HEIGHT.max}
                step={1}
                required
                aria-label={t("spec.resolution.height")}
                aria-describedby={`${id}-resolution-hint`}
                placeholder={t("spec.resolution.height")}
                value={height}
                onChange={(event) => set({ resolution: `${width}x${event.target.value}` })}
                className="min-w-0 flex-1"
              />
            </div>
            <FieldDescription id={`${id}-resolution-hint`} className="text-xs">
              {t("spec.resolution.customHint", {
                min: `${WIDTH.min}x${HEIGHT.min}`,
                max: `${WIDTH.max}x${HEIGHT.max}`,
              })}
            </FieldDescription>
          </>
        )}
      </Field>
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <Field>
          <FieldLabel htmlFor={`${id}-cpus`}>{t("spec.cpuLimit")}</FieldLabel>
          <Input
            id={`${id}-cpus`}
            type="number"
            min={0.1}
            step={0.1}
            placeholder={t("spec.unlimited")}
            value={draft.cpus}
            onChange={(event) => set({ cpus: event.target.value })}
          />
        </Field>
        <Field>
          <FieldLabel htmlFor={`${id}-memory`}>{t("spec.memoryLimit")}</FieldLabel>
          <Input
            id={`${id}-memory`}
            type="number"
            min={256}
            placeholder={t("spec.unlimited")}
            value={draft.memory_mb}
            onChange={(event) => set({ memory_mb: event.target.value })}
          />
        </Field>
        <Field>
          <FieldLabel htmlFor={`${id}-shm`}>{t("spec.sharedMemory")}</FieldLabel>
          <Input
            id={`${id}-shm`}
            type="number"
            min={64}
            required
            value={draft.shm_mb}
            onChange={(event) => set({ shm_mb: event.target.value })}
          />
        </Field>
      </div>
    </FieldGroup>
  )
}
