import { Field, FieldDescription, FieldLabel } from "@/components/ui/field"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { t } from "@/lib/i18n"
import { profileDescription, profileLabel, PROFILES } from "@/lib/mcp"
import type { McpProfile } from "@/lib/types"

/**
 * The tool profile of the endpoint a creation makes: which of its scopes' tools it offers. The
 * endpoint form changes it later.
 */
export function ProfileSelect({
  id,
  value,
  onChange,
}: {
  id: string
  value: McpProfile
  onChange: (profile: McpProfile) => void
}) {
  return (
    <Field>
      <FieldLabel htmlFor={id}>{t("mcp.grant.profile")}</FieldLabel>
      <Select value={value} onValueChange={(next) => onChange(next as McpProfile)}>
        <SelectTrigger id={id} className="w-full sm:w-64" aria-describedby={`${id}-hint`}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            {PROFILES.map((profile) => (
              <SelectItem key={profile} value={profile}>
                {profileLabel(profile)}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
      <FieldDescription id={`${id}-hint`} className="text-xs">
        {profileDescription(value)}
      </FieldDescription>
    </Field>
  )
}
