import * as React from "react"

import { Field, FieldDescription, FieldLabel, FieldSet } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { t, type MessageKey } from "@/lib/i18n"
import type { ContainerSpec } from "@/lib/types"

const FIELDS = ["hostname", "mac_address", "machine_id"] as const
type IdentityField = (typeof FIELDS)[number]
export type IdentityDraft = Record<IdentityField, string>

export function identityDraft(spec: Partial<ContainerSpec>): IdentityDraft {
  return {
    hostname: spec.hostname ?? "",
    mac_address: spec.mac_address ?? "",
    machine_id: spec.machine_id ?? "",
  }
}

/** The identity fields of a settings request. An empty field is left out: it cannot be cleared. */
export function identityBody(draft: IdentityDraft) {
  return Object.fromEntries(
    FIELDS.map((field) => [field, draft[field].trim()]).filter(([, value]) => value)
  )
}

const LABELS: Record<IdentityField, MessageKey> = {
  hostname: "identity.hostname",
  mac_address: "identity.macAddress",
  machine_id: "identity.machineId",
}

/**
 * The hostname, MAC address and machine ID a container keeps through resets and recreations.
 * `spec` is the container's: a value it has must stay filled, while a container created before
 * identities were kept shows where its values will come from.
 */
export function IdentityFields({
  draft,
  onChange,
  spec,
}: {
  draft: IdentityDraft
  onChange: (draft: IdentityDraft) => void
  spec: ContainerSpec
}) {
  const id = React.useId()
  // The section's heading names it, so the fields need no legend of their own.
  return (
    <FieldSet aria-label={t("identity.title")}>
      <FieldDescription className="text-xs">{t("identity.description")}</FieldDescription>
      {/* One per row: a hostname and a machine ID are too long to read in half the width. */}
      <div className="flex flex-col gap-4 sm:max-w-lg">
        {FIELDS.map((field) => (
          <Field key={field}>
            <FieldLabel htmlFor={`${id}-${field}`}>{t(LABELS[field])}</FieldLabel>
            <Input
              id={`${id}-${field}`}
              required={!!spec[field]}
              maxLength={field === "hostname" ? 63 : 40}
              autoComplete="off"
              spellCheck={false}
              placeholder={field === "machine_id" ? t("identity.fromImage") : t("identity.unset")}
              value={draft[field]}
              onChange={(event) => onChange({ ...draft, [field]: event.target.value })}
              className="font-mono"
            />
          </Field>
        ))}
      </div>
    </FieldSet>
  )
}
