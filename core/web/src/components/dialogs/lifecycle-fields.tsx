import * as React from "react"

import { Checkbox } from "@/components/ui/checkbox"
import { Field, FieldDescription, FieldLabel, FieldLegend, FieldSet } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { t } from "@/lib/i18n"
import { localTime, MAX_IDLE_MINUTES, type LifecycleDraft } from "@/lib/lifecycle"
import type { OnExpiry } from "@/lib/types"

/**
 * When Thoric stops or deletes the container by itself: an idle stop in minutes and an expiry.
 * `start` is the form as it opened for an existing container, whose expiry may have passed
 * already; only a time the form changes has to lie ahead.
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
  const [earliest] = React.useState(() => localTime(Math.floor(Date.now() / 1000) + 60))
  const set = (patch: Partial<LifecycleDraft>) => onChange({ ...draft, ...patch })
  const expires = draft.expires !== ""
  const ahead = !start || draft.expires !== start.expires
  return (
    <FieldSet>
      <FieldLegend variant="label">{t("lifecycle.title")}</FieldLegend>
      <FieldDescription className="text-xs">{t("lifecycle.description")}</FieldDescription>
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <Field>
          <FieldLabel htmlFor={`${id}-idle`}>{t("lifecycle.idle")}</FieldLabel>
          <Input
            id={`${id}-idle`}
            type="number"
            min={0}
            max={MAX_IDLE_MINUTES}
            step={1}
            placeholder={t("lifecycle.never")}
            aria-describedby={`${id}-idle-hint`}
            value={draft.idle}
            onChange={(event) => set({ idle: event.target.value })}
          />
          <FieldDescription id={`${id}-idle-hint`} className="text-xs">
            {t("lifecycle.idleHint")}
          </FieldDescription>
        </Field>
        <Field>
          <FieldLabel htmlFor={`${id}-expires`}>{t("lifecycle.expires")}</FieldLabel>
          <Input
            id={`${id}-expires`}
            type="datetime-local"
            min={ahead ? earliest : undefined}
            aria-describedby={`${id}-expires-hint`}
            value={draft.expires}
            onChange={(event) => set({ expires: event.target.value })}
          />
          <FieldDescription id={`${id}-expires-hint`} className="text-xs">
            {t("lifecycle.expiresHint")}
          </FieldDescription>
        </Field>
        {expires && (
          <Field className="sm:col-span-2">
            <FieldLabel htmlFor={`${id}-on-expiry`}>{t("lifecycle.onExpiry")}</FieldLabel>
            <Select
              value={draft.onExpiry}
              onValueChange={(value) => set({ onExpiry: value as OnExpiry })}
            >
              <SelectTrigger id={`${id}-on-expiry`} className="w-full sm:w-64">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem value="stop">{t("lifecycle.onExpiry.stop")}</SelectItem>
                  <SelectItem value="delete">{t("lifecycle.onExpiry.delete")}</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
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
      </div>
    </FieldSet>
  )
}
