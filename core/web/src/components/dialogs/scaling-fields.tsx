import * as React from "react"
import { RiStackLine } from "@remixicon/react"

import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { t } from "@/lib/i18n"
import { IDLE_PRESETS, MAX_IDLE_MINUTES, minutesText } from "@/lib/lifecycle"
import {
  MAX_INSTANCES,
  POLICIES,
  SET_NAME_PATTERN,
  setNameOf,
  type ScalingDraft,
  type VolumePolicy,
} from "@/lib/scale"
import { choiceGroup, choiceItem } from "./screen-fields"
import { SwitchTile } from "./spec-fields"

const NEVER = "never"
const CUSTOM = "custom"

/**
 * How a container is made scalable: the set's name, how many instances it keeps and may have,
 * when an idle one goes, and what each volume becomes in a new instance. `toggle` shows the switch
 * that turns scaling on, as the create form does; `targets` are the folders its volumes are
 * mounted at; `name` the container's, which the set is named after when its own is left empty.
 * `named` leaves the name and the introduction out, for a set that exists, whose name stays.
 */
export function ScalingFields({
  draft,
  onChange,
  targets,
  name,
  toggle = false,
  named = false,
}: {
  draft: ScalingDraft
  onChange: (draft: ScalingDraft) => void
  targets: string[]
  name: string
  toggle?: boolean
  named?: boolean
}) {
  const id = React.useId()
  const set = (patch: Partial<ScalingDraft>) => onChange({ ...draft, ...patch })
  const shown = !toggle || draft.enabled
  // The idle delete offers the lengths the idle stop does; one off the list is custom, and a
  // listed one only once Custom is picked, so typing a listed length does not close the field.
  const idle = Number(draft.idle) || 0
  const [idlePicked, setIdlePicked] = React.useState(() => !!idle && !IDLE_PRESETS.includes(idle))
  const idleCustom = idlePicked || (!!idle && !IDLE_PRESETS.includes(idle))
  const idleChoice = idleCustom ? CUSTOM : idle ? String(idle) : NEVER

  return (
    <FieldGroup>
      {toggle ? (
        <Field>
          <SwitchTile
            icon={RiStackLine}
            label={t("scale.enable")}
            checked={draft.enabled}
            onCheckedChange={(enabled) => set({ enabled })}
            describedBy={`${id}-hint`}
          />
          <FieldDescription id={`${id}-hint`} className="text-xs">
            {t("scale.enableHint")}
          </FieldDescription>
        </Field>
      ) : (
        !named && <FieldDescription className="text-xs">{t("scale.enableHint")}</FieldDescription>
      )}
      {shown && (
        <>
          {!named && (
            <Field className="sm:max-w-sm">
              <FieldLabel htmlFor={`${id}-set`}>{t("scale.setName")}</FieldLabel>
              <Input
                id={`${id}-set`}
                pattern={SET_NAME_PATTERN}
                maxLength={40}
                title={t("scale.setNamePattern")}
                placeholder={setNameOf(name)}
                value={draft.set}
                onChange={(event) => set({ set: event.target.value })}
                className="font-mono"
              />
              <FieldDescription className="text-xs">
                {t("scale.setNameHint", { name: draft.set.trim() || setNameOf(name) })}
              </FieldDescription>
            </Field>
          )}
          {/* The two counts side by side, their fields level whatever a label's length. */}
          <div className="flex flex-col gap-2">
            <div className="grid grid-cols-2 items-end gap-4 sm:max-w-sm">
              <Field>
                <FieldLabel htmlFor={`${id}-min`}>{t("scale.min")}</FieldLabel>
                <Input
                  id={`${id}-min`}
                  type="number"
                  min={0}
                  max={MAX_INSTANCES}
                  step={1}
                  required
                  value={draft.min}
                  onChange={(event) => set({ min: event.target.value })}
                />
              </Field>
              <Field>
                <FieldLabel htmlFor={`${id}-max`}>{t("scale.max")}</FieldLabel>
                <Input
                  id={`${id}-max`}
                  type="number"
                  min={1}
                  max={MAX_INSTANCES}
                  step={1}
                  required
                  value={draft.max}
                  onChange={(event) => set({ max: event.target.value })}
                />
              </Field>
            </div>
            <FieldDescription className="text-xs">{t("scale.limitsHint")}</FieldDescription>
          </div>
          <Field>
            <FieldLabel id={`${id}-idle`}>{t("scale.idleDelete")}</FieldLabel>
            <ToggleGroup
              type="single"
              variant="outline"
              size="sm"
              aria-labelledby={`${id}-idle`}
              aria-describedby={`${id}-idle-hint`}
              value={idleChoice}
              onValueChange={(value) => {
                if (!value) return
                setIdlePicked(value === CUSTOM)
                if (value === NEVER) set({ idle: "0" })
                else if (value !== CUSTOM) set({ idle: value })
              }}
              className={choiceGroup}
            >
              <ToggleGroupItem value={NEVER} className={choiceItem}>
                {t("lifecycle.never")}
              </ToggleGroupItem>
              {IDLE_PRESETS.map((value) => (
                <ToggleGroupItem key={value} value={String(value)} className={choiceItem}>
                  {minutesText(value)}
                </ToggleGroupItem>
              ))}
              <ToggleGroupItem value={CUSTOM} className={choiceItem}>
                {t("lifecycle.custom")}
              </ToggleGroupItem>
            </ToggleGroup>
            {idleCustom && (
              <Input
                type="number"
                min={1}
                max={MAX_IDLE_MINUTES}
                step={1}
                required
                aria-label={t("lifecycle.minutes")}
                placeholder={t("lifecycle.minutes")}
                value={draft.idle === "0" ? "" : draft.idle}
                onChange={(event) => set({ idle: event.target.value })}
                className="sm:max-w-48"
              />
            )}
            <FieldDescription id={`${id}-idle-hint`} className="text-xs">
              {t("scale.idleDeleteHint")}
            </FieldDescription>
          </Field>
          <Field>
            <FieldLabel>{t("scale.volumes")}</FieldLabel>
            {targets.length ? (
              <ul className="flex flex-col divide-y rounded-xl border">
                {targets.map((target) => (
                  <li
                    key={target}
                    className="flex flex-col gap-2 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between"
                  >
                    <code className="min-w-0 font-mono text-xs break-all">{target}</code>
                    <Select
                      value={draft.volumes[target] ?? "shared"}
                      onValueChange={(value) =>
                        set({ volumes: { ...draft.volumes, [target]: value as VolumePolicy } })
                      }
                    >
                      <SelectTrigger
                        size="sm"
                        className="w-full sm:w-56"
                        aria-label={t("scale.policyOf", { target })}
                      >
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectGroup>
                          {POLICIES.map((policy) => (
                            <SelectItem key={policy} value={policy}>
                              {t(`scale.policy.${policy}`)}
                            </SelectItem>
                          ))}
                        </SelectGroup>
                      </SelectContent>
                    </Select>
                  </li>
                ))}
              </ul>
            ) : (
              <FieldDescription className="text-xs">{t("scale.noVolumes")}</FieldDescription>
            )}
            <FieldDescription className="text-xs">{t("scale.volumesHint")}</FieldDescription>
          </Field>
        </>
      )}
    </FieldGroup>
  )
}
