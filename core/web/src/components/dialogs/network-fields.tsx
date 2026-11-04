import * as React from "react"
import { RiAlertLine } from "@remixicon/react"

import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { TagInput } from "@/components/ui/tag-input"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { type KnownGroup, LABEL_PATTERN } from "@/lib/groups"
import { plural, rich, t } from "@/lib/i18n"
import { PORTS_PATTERN } from "@/lib/ports"
import { PROXY_URL_PATTERN, type ProxyDraft, type ProxyMode } from "@/lib/proxy"
import type { Settings } from "@/lib/types"
import { useShowSection } from "./form-sections"
import { choiceGroup, choiceItem } from "./screen-fields"
import type { SpecDraft } from "./spec-fields"

const lowercase = (text: string) => text.toLowerCase()

/** How many containers list a group, beside its suggestion. */
function memberCount(groups: readonly KnownGroup[], name: string) {
  const count = groups.find((group) => group.name === name)?.members.length ?? 0
  return plural(count, { one: "groups.members.one", other: "groups.members.other" })
}

/**
 * Who reaches the desktop and how it reaches out: its proxied ports, its groups and the names it
 * answers to there, and its proxy. Internet access is a feature, so this only says where it is.
 * `groups` are the groups other containers list, and `vncPrivate` whether the container's screen
 * stays away from the members of its groups. `basics` is the section with the features.
 */
export function NetworkFields({
  draft,
  onChange,
  settings,
  groups,
  vncPrivate,
  basics,
}: {
  draft: SpecDraft
  onChange: (draft: SpecDraft) => void
  settings?: Settings
  groups: KnownGroup[]
  vncPrivate?: boolean | null
  basics: string
}) {
  const id = React.useId()
  const show = useShowSection()
  const set = (patch: Partial<SpecDraft>) => onChange({ ...draft, ...patch })

  return (
    <FieldGroup>
      <FieldDescription className="text-xs">
        {rich(draft.internet ? "spec.internetOn" : "spec.internetOff", {
          change: (
            <Button
              type="button"
              variant="link"
              className="h-auto p-0 text-xs"
              onClick={() => show(basics)}
            >
              {t("spec.changeUnderBasics")}
            </Button>
          ),
        })}
      </FieldDescription>
      <Field>
        <FieldLabel htmlFor={`${id}-ports`}>{t("spec.proxiedPorts")}</FieldLabel>
        <Input
          id={`${id}-ports`}
          inputMode="numeric"
          pattern={PORTS_PATTERN}
          placeholder="8090, 6400"
          aria-describedby={`${id}-ports-hint`}
          value={draft.ports}
          onChange={(event) => set({ ports: event.target.value })}
          className="font-mono"
        />
        <FieldDescription id={`${id}-ports-hint`} className="text-xs">
          {t("spec.proxiedPortsHint")}
        </FieldDescription>
      </Field>
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <Field>
          <FieldLabel htmlFor={`${id}-groups`}>{t("spec.groups")}</FieldLabel>
          <TagInput
            id={`${id}-groups`}
            value={draft.groups}
            onChange={(next) => set({ groups: next })}
            suggestions={groups.map((group) => group.name)}
            detail={(name) => memberCount(groups, name)}
            pattern={LABEL_PATTERN}
            normalize={lowercase}
            placeholder={t("spec.groupsAdd")}
            aria-describedby={`${id}-groups-hint`}
          />
          <FieldDescription id={`${id}-groups-hint`} className="text-xs">
            {t("spec.groupsHint")}
          </FieldDescription>
        </Field>
        <Field>
          <FieldLabel htmlFor={`${id}-aliases`}>{t("spec.aliases")}</FieldLabel>
          <TagInput
            id={`${id}-aliases`}
            value={draft.aliases}
            onChange={(aliases) => set({ aliases })}
            pattern={LABEL_PATTERN}
            normalize={lowercase}
            placeholder={t("spec.aliasesAdd")}
            aria-describedby={`${id}-aliases-hint`}
          />
          <FieldDescription id={`${id}-aliases-hint`} className="text-xs">
            {t("spec.aliasesHint")}
          </FieldDescription>
        </Field>
      </div>
      {vncPrivate === false && draft.groups.length > 0 && (
        <Alert>
          <RiAlertLine className="text-warning" />
          <AlertDescription>{t("spec.groupsExposed")}</AlertDescription>
        </Alert>
      )}
      <ProxyFields draft={draft.proxy} set={(proxy) => set({ proxy })} settings={settings} />
    </FieldGroup>
  )
}

/**
 * The proxy the desktop's programs go through: none, the installation's, which is offered while
 * the installation has one or the container follows it, or one of its own with its credentials.
 */
function ProxyFields({
  draft,
  set,
  settings,
}: {
  draft: ProxyDraft
  set: (proxy: ProxyDraft) => void
  settings?: Settings
}) {
  const id = React.useId()
  const update = (patch: Partial<ProxyDraft>) => set({ ...draft, ...patch })
  const installation = settings?.proxy
  const modes: ProxyMode[] =
    installation || draft.mode === "installation"
      ? ["none", "installation", "custom"]
      : ["none", "custom"]
  const description =
    draft.mode === "none"
      ? t("spec.proxy.none.description")
      : draft.mode === "custom"
        ? t("spec.proxy.custom.description")
        : installation
          ? t("spec.proxy.installation.description", { url: installation.url })
          : t("spec.proxy.installation.missing")

  return (
    <FieldSet>
      <FieldLegend variant="label" className="mb-0">
        {t("spec.proxy")}
      </FieldLegend>
      <Field>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-label={t("spec.proxy")}
          aria-describedby={`${id}-hint`}
          value={draft.mode}
          onValueChange={(mode) => {
            if (mode) update({ mode: mode as ProxyMode })
          }}
          className={choiceGroup}
        >
          {modes.map((mode) => (
            <ToggleGroupItem key={mode} value={mode} className={choiceItem}>
              {t(`spec.proxy.mode.${mode}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <FieldDescription id={`${id}-hint`} className="text-xs break-words">
          {description}
        </FieldDescription>
      </Field>
      {draft.mode === "custom" && (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <Field className="sm:col-span-2">
            <FieldLabel htmlFor={`${id}-url`}>{t("spec.proxy.url")}</FieldLabel>
            <Input
              id={`${id}-url`}
              required
              pattern={PROXY_URL_PATTERN}
              placeholder="http://proxy.example:3128"
              autoComplete="off"
              value={draft.url}
              onChange={(event) => update({ url: event.target.value })}
              className="font-mono"
            />
          </Field>
          <Field>
            <FieldLabel htmlFor={`${id}-username`}>{t("spec.proxy.username")}</FieldLabel>
            <Input
              id={`${id}-username`}
              maxLength={255}
              autoComplete="off"
              value={draft.username}
              onChange={(event) => update({ username: event.target.value })}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor={`${id}-password`}>{t("spec.proxy.password")}</FieldLabel>
            <Input
              id={`${id}-password`}
              type="password"
              maxLength={255}
              autoComplete="new-password"
              placeholder={draft.stored && !draft.clear ? t("spec.proxy.passwordKept") : undefined}
              value={draft.password}
              onChange={(event) => update({ password: event.target.value, clear: false })}
            />
            {draft.stored && (
              <Label className="flex items-center gap-2 text-xs font-normal">
                <Checkbox
                  checked={draft.clear}
                  onCheckedChange={(value) => update({ clear: !!value, password: "" })}
                />
                {t("spec.proxy.removePassword")}
              </Label>
            )}
          </Field>
          <Field className="sm:col-span-2">
            <FieldLabel htmlFor={`${id}-bypass`}>{t("spec.proxy.bypass")}</FieldLabel>
            <Input
              id={`${id}-bypass`}
              placeholder="intranet.example, 10.0.0.0/8"
              autoComplete="off"
              value={draft.bypass}
              onChange={(event) => update({ bypass: event.target.value })}
              className="font-mono"
            />
          </Field>
        </div>
      )}
    </FieldSet>
  )
}
