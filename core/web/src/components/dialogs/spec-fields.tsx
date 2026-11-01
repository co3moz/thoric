import * as React from "react"
import { flushSync } from "react-dom"
import {
  RiAddLine,
  RiAlertLine,
  RiArrowDownSLine,
  RiCloseLine,
  RiCpuLine,
  RiGamepadLine,
  RiGlobalLine,
  RiMicLine,
  RiShieldKeyholeLine,
  RiShieldUserLine,
  RiVolumeUpLine,
  type RemixiconComponentType,
} from "@remixicon/react"
import { cn } from "cn"
import { toast } from "sonner"

import { ConceptIcon } from "@/components/concept"
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
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { TagInput } from "@/components/ui/tag-input"
import { Toggle } from "@/components/ui/toggle"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { type KnownGroup, LABEL_PATTERN } from "@/lib/groups"
import { formatList, formatNumber, plural, t } from "@/lib/i18n"
import { formatPorts, parsePorts, PORTS_PATTERN } from "@/lib/ports"
import {
  PROXY_URL_PATTERN,
  proxyBody,
  proxyDraft,
  type ProxyDraft,
  type ProxyMode,
} from "@/lib/proxy"
import { featureDraft, FEATURES, type Feature } from "@/lib/settings"
import type { ContainerSpec, Mount, Settings, Volume } from "@/lib/types"
import { mountableVolumes } from "@/lib/volume-files"

const RESOLUTIONS = ["1920x1080", "1600x900", "1280x720", "2560x1440", "3840x2160"]
/** The select's entry for a size of one's own, which no `WIDTHxHEIGHT` can be. */
const CUSTOM = "custom"
/** The sides the manager accepts (`ContainerSpec::validate`). */
const WIDTH = { min: 320, max: 7680 }
const HEIGHT = { min: 240, max: 4320 }
const FLAGS = FEATURES
type Flag = Feature

const FLAG_ICONS: Record<Flag, RemixiconComponentType> = {
  gpu: RiCpuLine,
  internet: RiGlobalLine,
  sudo: RiShieldUserLine,
  audio: RiVolumeUpLine,
  microphone: RiMicLine,
  gamepad: RiGamepadLine,
  vpn: RiShieldKeyholeLine,
}

export type MountDraft = Mount & { key: number }
export type SpecDraft = {
  resolution: string
  shm_mb: string
  cpus: string
  memory_mb: string
  ports: string
  groups: string[]
  aliases: string[]
  mounts: MountDraft[]
  proxy: ProxyDraft
} & Record<Flag, boolean>

let mountKey = 0

/** The draft of a container's settings, or for a new one the installation's defaults. */
export function specDraft(spec: Partial<ContainerSpec>, settings?: Settings): SpecDraft {
  return {
    resolution: spec.resolution || RESOLUTIONS[0],
    shm_mb: String(spec.shm_mb ?? 512),
    cpus: spec.cpus ? String(spec.cpus) : "",
    memory_mb: spec.memory_mb ? String(spec.memory_mb) : "",
    ...featureDraft(spec, settings),
    ports: formatPorts(spec.ports),
    groups: spec.groups ?? [],
    aliases: spec.aliases ?? [],
    mounts: (spec.mounts ?? []).map((mount) => ({ ...mount, key: ++mountKey })),
    proxy: proxyDraft(spec, settings),
  }
}

/** The request body for the settings shared by creation and later changes. */
export function specBody(draft: SpecDraft) {
  const number = (value: string) => (value ? Number(value) : null)
  return {
    resolution: draft.resolution,
    shm_mb: number(draft.shm_mb),
    cpus: number(draft.cpus),
    memory_mb: number(draft.memory_mb),
    ...Object.fromEntries(FLAGS.map((flag) => [flag, draft[flag]])),
    ports: parsePorts(draft.ports),
    groups: draft.groups,
    aliases: draft.aliases,
    mounts: draft.mounts.map(({ volume, target, readonly }) => ({ volume, target, readonly })),
    ...proxyBody(draft.proxy),
  }
}

const flagLabel = (flag: Flag) => t(`spec.flag.${flag}`)
const flagDescription = (flag: Flag) => t(`spec.flag.${flag}.description`)

const lowercase = (text: string) => text.toLowerCase()

/** How many containers list a group, beside its suggestion. */
function memberCount(groups: readonly KnownGroup[], name: string) {
  const count = groups.find((group) => group.name === name)?.members.length ?? 0
  return plural(count, { one: "groups.members.one", other: "groups.members.other" })
}

/** One line for the closed advanced section, so its values are known without opening it. */
function advancedSummary(draft: SpecDraft) {
  const parts = [
    draft.resolution,
    draft.cpus
      ? t("spec.cpus", { count: formatNumber(Number(draft.cpus)) })
      : t("spec.unlimitedCpu"),
    draft.memory_mb
      ? t("spec.memory", { count: formatNumber(Number(draft.memory_mb)) })
      : t("spec.unlimitedMemory"),
  ]
  const ports = parsePorts(draft.ports).length
  if (ports) parts.push(plural(ports, { one: "spec.ports.one", other: "spec.ports.other" }))
  const groups = draft.groups.length
  if (groups) parts.push(plural(groups, { one: "spec.groups.one", other: "spec.groups.other" }))
  if (draft.mounts.length)
    parts.push(plural(draft.mounts.length, { one: "spec.mounts.one", other: "spec.mounts.other" }))
  if (draft.proxy.mode !== "none") parts.push(t("spec.proxy.summary"))
  return parts.join(" · ")
}

/**
 * The settings shared by the create and settings dialogs: the features as toggle buttons, then
 * resources and volumes in a section that starts closed unless `advancedOpen`. `spec` is the
 * container being changed, if any: a feature the installation switched off cannot be turned on, but
 * a container that has it can still turn it off. `groups` are the groups other containers list,
 * and `vncPrivate` whether the container's screen stays away from the members of its groups.
 */
export function SpecFields({
  draft,
  onChange,
  settings,
  volumes,
  spec,
  groups = [],
  vncPrivate,
  advancedOpen = false,
}: {
  draft: SpecDraft
  onChange: (draft: SpecDraft) => void
  settings?: Settings
  volumes: Volume[]
  spec?: ContainerSpec
  groups?: KnownGroup[]
  vncPrivate?: boolean | null
  advancedOpen?: boolean
}) {
  const id = React.useId()
  const [open, setOpen] = React.useState(advancedOpen)
  // A size off the list is custom; a listed one only once Custom is picked, so typing a listed
  // size into the fields does not close them.
  const [customPicked, setCustomPicked] = React.useState(
    () => !RESOLUTIONS.includes(draft.resolution)
  )
  const custom = customPicked || !RESOLUTIONS.includes(draft.resolution)
  const [width = "", height = ""] = draft.resolution.split("x")
  const enabled: Record<Flag, boolean> = {
    gpu: settings?.gpu_enabled ?? true,
    audio: settings?.audio_enabled ?? true,
    microphone: settings?.microphone_enabled ?? true,
    internet: settings?.internet_enabled ?? true,
    sudo: settings?.sudo_enabled ?? true,
    gamepad: settings?.gamepad_enabled ?? true,
    vpn: settings?.vpn_enabled ?? true,
  }
  const blocked = FLAGS.filter((flag) => !enabled[flag] && !spec?.[flag])
  // The settings allow a GPU, but the host has none of the ones they allow.
  const noGpu = blocked.includes("gpu") && !!settings?.gpu?.unused.length
  const turnedOff = noGpu ? blocked.filter((flag) => flag !== "gpu") : blocked
  const set = (patch: Partial<SpecDraft>) => onChange({ ...draft, ...patch })
  const toggle = (flag: Flag, value: boolean) => {
    const patch: Partial<SpecDraft> = { [flag]: value }
    // The microphone travels over the audio connection.
    if (flag === "microphone" && value) patch.audio = true
    if (flag === "audio" && !value) patch.microphone = false
    set(patch)
  }

  return (
    <FieldGroup>
      <FieldSet className="gap-3">
        <FieldLegend variant="label">{t("spec.features")}</FieldLegend>
        <div className="flex flex-wrap gap-2">
          {FLAGS.map((flag) => {
            const Icon = FLAG_ICONS[flag]
            const disabled = blocked.includes(flag)
            const button = (
              <Toggle
                key={flag}
                variant="outline"
                pressed={draft[flag]}
                disabled={disabled}
                aria-describedby={disabled ? `${id}-blocked` : undefined}
                onPressedChange={(value) => toggle(flag, value)}
                // The tooltip trigger overrides data-state, so aria-pressed carries the style.
                className="text-muted-foreground aria-pressed:border-primary aria-pressed:text-foreground"
              >
                <Icon data-icon="inline-start" />
                {flagLabel(flag)}
              </Toggle>
            )
            // A disabled button gets no hover or focus, so the line below says why instead.
            if (disabled) return button
            return (
              <Tooltip key={flag}>
                <TooltipTrigger asChild>{button}</TooltipTrigger>
                <TooltipContent>{flagDescription(flag)}</TooltipContent>
              </Tooltip>
            )
          })}
        </div>
        {blocked.length > 0 && (
          <FieldDescription id={`${id}-blocked`} className="text-xs">
            {[
              turnedOff.length > 0 &&
                t("spec.turnedOffList", { features: formatList(turnedOff.map(flagLabel)) }),
              noGpu && t("spec.gpuUnavailable"),
            ]
              .filter(Boolean)
              .join(" ")}
          </FieldDescription>
        )}
      </FieldSet>
      {/* Not a Radix collapsible: that unmounts its content when closed, and the browser could
          then no longer check the fields inside before submitting. */}
      <div
        className="rounded-2xl border"
        // The browser points at a field it refuses only when the field is visible, so a closed
        // section opens before the browser looks for the field to focus.
        onInvalidCapture={() => {
          if (!open) flushSync(() => setOpen(true))
        }}
      >
        <button
          type="button"
          aria-expanded={open}
          aria-controls={`${id}-advanced`}
          onClick={() => setOpen(!open)}
          className="group flex w-full items-center gap-3 rounded-2xl px-4 py-3 text-left outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
        >
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="text-sm font-medium group-hover:underline">{t("spec.advanced")}</span>
            {!open && (
              <span className="text-xs text-muted-foreground">{advancedSummary(draft)}</span>
            )}
          </span>
          <RiArrowDownSLine
            className={cn(
              "size-4 shrink-0 text-muted-foreground transition-transform",
              open && "rotate-180"
            )}
          />
        </button>
        <div id={`${id}-advanced`} hidden={!open} className="border-t p-4">
          <FieldGroup>
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
              <Field>
                <FieldLabel htmlFor={`${id}-resolution`}>{t("spec.resolution")}</FieldLabel>
                <Select
                  value={custom ? CUSTOM : draft.resolution}
                  onValueChange={(value) => {
                    setCustomPicked(value === CUSTOM)
                    if (value !== CUSTOM) set({ resolution: value })
                  }}
                >
                  <SelectTrigger id={`${id}-resolution`} className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      {RESOLUTIONS.map((value) => (
                        <SelectItem key={value} value={value}>
                          {value}
                        </SelectItem>
                      ))}
                      <SelectItem value={CUSTOM}>{t("spec.resolution.custom")}</SelectItem>
                    </SelectGroup>
                  </SelectContent>
                </Select>
                {custom && (
                  <>
                    <div className="flex items-center gap-2">
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
              <Field className="sm:col-span-2">
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
              <Field className="sm:col-span-2">
                <FieldLabel htmlFor={`${id}-groups`}>{t("spec.groups")}</FieldLabel>
                <TagInput
                  id={`${id}-groups`}
                  value={draft.groups}
                  onChange={(groups) => set({ groups })}
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
                {vncPrivate === false && draft.groups.length > 0 && (
                  <Alert>
                    <RiAlertLine className="text-warning" />
                    <AlertDescription>{t("spec.groupsExposed")}</AlertDescription>
                  </Alert>
                )}
              </Field>
              <Field className="sm:col-span-2">
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
            <ProxyFields draft={draft.proxy} set={(proxy) => set({ proxy })} settings={settings} />
            <Mounts draft={draft} set={set} volumes={mountableVolumes(volumes)} />
          </FieldGroup>
        </div>
      </div>
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
        <Select value={draft.mode} onValueChange={(mode) => update({ mode: mode as ProxyMode })}>
          <SelectTrigger
            aria-label={t("spec.proxy")}
            aria-describedby={`${id}-hint`}
            className="w-full sm:w-64"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {modes.map((mode) => (
                <SelectItem key={mode} value={mode}>
                  {t(`spec.proxy.mode.${mode}`)}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
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

/**
 * The volumes mounted into the container, each with its path and access. `volumes` are the ones a
 * container may mount.
 */
function Mounts({
  draft,
  set,
  volumes,
}: {
  draft: SpecDraft
  set: (patch: Partial<SpecDraft>) => void
  volumes: Volume[]
}) {
  const setMount = (key: number, patch: Partial<Mount>) =>
    set({
      mounts: draft.mounts.map((mount) => (mount.key === key ? { ...mount, ...patch } : mount)),
    })

  return (
    <FieldSet>
      <div className="flex items-center justify-between">
        <FieldLegend variant="label" className="mb-0">
          {t("nav.volumes")}
        </FieldLegend>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            if (!volumes.length) {
              toast.error(t("spec.volumeFirst"))
              return
            }
            set({
              mounts: [
                ...draft.mounts,
                { key: ++mountKey, volume: volumes[0].Name, target: "", readonly: false },
              ],
            })
          }}
        >
          <RiAddLine data-icon="inline-start" />
          {t("spec.mount")}
        </Button>
      </div>
      {draft.mounts.map((mount) => {
        const names = volumes.map((volume) => volume.Name)
        // A container's existing mount stays visible, so it can be removed or changed.
        if (!names.includes(mount.volume)) names.push(mount.volume)
        return (
          <div
            key={mount.key}
            className="grid grid-cols-[1fr_auto_auto] items-center gap-2 sm:grid-cols-[10rem_1fr_auto_auto]"
          >
            <Select
              value={mount.volume}
              onValueChange={(volume) => setMount(mount.key, { volume })}
            >
              <SelectTrigger aria-label={t("spec.volume")} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {names.map((name) => (
                    <SelectItem key={name} value={name}>
                      <ConceptIcon concept="volume" />
                      {name}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
            <Input
              aria-label={t("spec.mountPath")}
              placeholder="/workspace/project"
              required
              value={mount.target}
              onChange={(event) => setMount(mount.key, { target: event.target.value })}
              className="col-span-3 row-start-2 font-mono sm:col-span-1 sm:row-start-auto"
            />
            <Label className="flex items-center gap-2 text-xs font-normal">
              <Checkbox
                checked={mount.readonly}
                onCheckedChange={(value) => setMount(mount.key, { readonly: !!value })}
              />
              {t("spec.readOnly")}
            </Label>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label={t("spec.removeMount")}
              onClick={() => set({ mounts: draft.mounts.filter((item) => item.key !== mount.key) })}
            >
              <RiCloseLine />
            </Button>
          </div>
        )
      })}
      <FieldDescription className="text-xs">{t("spec.mountsHint")}</FieldDescription>
    </FieldSet>
  )
}
