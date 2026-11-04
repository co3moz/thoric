import * as React from "react"

import { Checkbox } from "@/components/ui/checkbox"
import { Field, FieldDescription, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { formatDate, t } from "@/lib/i18n"
import {
  EXPIRY_PRESETS,
  IDLE_PRESETS,
  localTime,
  MAX_IDLE_MINUTES,
  minutesText,
  type LifecycleDraft,
} from "@/lib/lifecycle"
import type { OnExpiry } from "@/lib/types"
import { choiceGroup, choiceItem } from "./screen-fields"

const NEVER = "never"
const CUSTOM = "custom"

/**
 * When Thoric stops or deletes the container by itself: an idle stop and an expiry, each a few
 * common lengths or one of one's own. `start` is the form as it opened for an existing container,
 * whose expiry may have passed already; only a time the form changes has to lie ahead.
 */
export function LifecycleFields({
  draft,
  onChange,
  start,
}: {
  draft: LifecycleDraft
  onChange: (draft: LifecycleDraft) => void
  start?: LifecycleDraft
}) {
  const id = React.useId()
  // The earliest expiry the field offers: a minute after the form opened.
  const [opened] = React.useState(() => Math.floor(Date.now() / 1000))
  const earliest = localTime(opened + 60)
  const set = (patch: Partial<LifecycleDraft>) => onChange({ ...draft, ...patch })
  const ahead = !start || draft.expires !== start.expires

  // A length off the list is custom; a listed one only once Custom is picked, so typing a
  // listed length does not close the field.
  const idle = Number(draft.idle) || 0
  const [idlePicked, setIdlePicked] = React.useState(() => !!idle && !IDLE_PRESETS.includes(idle))
  const idleCustom = idlePicked || (!!idle && !IDLE_PRESETS.includes(idle))
  const idleChoice = idleCustom ? CUSTOM : idle ? String(idle) : NEVER

  const [datePicked, setDatePicked] = React.useState(() => draft.expires !== "")
  const expiryChoice = draft.expiresIn
    ? draft.expiresIn
    : datePicked || draft.expires
      ? CUSTOM
      : NEVER
  const expiring = expiryChoice !== NEVER
  const minutes = Number(draft.expiresIn) || 0

  return (
    <FieldGroup>
      <FieldDescription className="text-xs">{t("lifecycle.description")}</FieldDescription>
      <Field>
        <FieldLabel id={`${id}-idle`}>{t("lifecycle.idleFor")}</FieldLabel>
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
            if (value === NEVER) set({ idle: "" })
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
            value={draft.idle}
            onChange={(event) => set({ idle: event.target.value })}
            className="sm:max-w-48"
          />
        )}
        <FieldDescription id={`${id}-idle-hint`} className="text-xs">
          {t("lifecycle.idleHint")}
        </FieldDescription>
      </Field>
      <Field>
        <FieldLabel id={`${id}-expires`}>{t("lifecycle.expires")}</FieldLabel>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-labelledby={`${id}-expires`}
          value={expiryChoice}
          onValueChange={(value) => {
            if (!value) return
            setDatePicked(value === CUSTOM)
            if (value === NEVER) set({ expires: "", expiresIn: "" })
            else if (value === CUSTOM) set({ expiresIn: "" })
            else set({ expires: "", expiresIn: value })
          }}
          className={choiceGroup}
        >
          <ToggleGroupItem value={NEVER} className={choiceItem}>
            {t("lifecycle.never")}
          </ToggleGroupItem>
          {EXPIRY_PRESETS.map((value) => (
            <ToggleGroupItem key={value} value={String(value)} className={choiceItem}>
              {t("lifecycle.in", { duration: minutesText(value) })}
            </ToggleGroupItem>
          ))}
          <ToggleGroupItem value={CUSTOM} className={choiceItem}>
            {t("lifecycle.pickDate")}
          </ToggleGroupItem>
        </ToggleGroup>
        {expiryChoice === CUSTOM && (
          <Input
            type="datetime-local"
            required
            min={ahead ? earliest : undefined}
            aria-label={t("lifecycle.expires")}
            value={draft.expires}
            onChange={(event) => set({ expires: event.target.value, expiresIn: "" })}
            className="sm:max-w-64"
          />
        )}
        {minutes > 0 && (
          <FieldDescription className="text-xs">
            {t("lifecycle.expiresAround", {
              date: formatDate(opened + minutes * 60, { dateStyle: "medium", timeStyle: "short" }),
            })}
          </FieldDescription>
        )}
      </Field>
      {expiring && (
        <Field>
          <FieldLabel id={`${id}-on-expiry`}>{t("lifecycle.onExpiry")}</FieldLabel>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            aria-labelledby={`${id}-on-expiry`}
            value={draft.onExpiry}
            onValueChange={(value) => {
              if (value) set({ onExpiry: value as OnExpiry })
            }}
            className={choiceGroup}
          >
            <ToggleGroupItem value="stop" className={choiceItem}>
              {t("lifecycle.onExpiry.stop")}
            </ToggleGroupItem>
            <ToggleGroupItem value="delete" className={choiceItem}>
              {t("lifecycle.onExpiry.delete")}
            </ToggleGroupItem>
          </ToggleGroup>
          <FieldDescription className="text-xs">
            {draft.onExpiry === "delete"
              ? t("lifecycle.onExpiry.deleteHint")
              : t("lifecycle.onExpiry.stopHint")}
          </FieldDescription>
          {draft.onExpiry === "delete" && (
            <Label className="flex items-center gap-2 text-xs font-normal">
              <Checkbox
                checked={draft.deleteVolumes}
                onCheckedChange={(value) => set({ deleteVolumes: value === true })}
              />
              {t("lifecycle.deleteVolumes")}
            </Label>
          )}
        </Field>
      )}
    </FieldGroup>
  )
}
