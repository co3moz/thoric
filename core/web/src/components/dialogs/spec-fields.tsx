import * as React from "react"
import {
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

import { FieldDescription, FieldLegend, FieldSet } from "@/components/ui/field"
import { Switch } from "@/components/ui/switch"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { formatList, t } from "@/lib/i18n"
import { formatPorts, parsePorts } from "@/lib/ports"
import { proxyBody, proxyDraft, type ProxyDraft } from "@/lib/proxy"
import { featureDraft, FEATURES, type Feature } from "@/lib/settings"
import type { ContainerSpec, Mount, Settings } from "@/lib/types"

export const RESOLUTIONS = ["1920x1080", "1600x900", "1280x720", "2560x1440", "3840x2160"]
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
  tmp_mb: string
  cpus: string
  memory_mb: string
  ports: string
  groups: string[]
  aliases: string[]
  mounts: MountDraft[]
  proxy: ProxyDraft
} & Record<Flag, boolean>

let mountKey = 0

/** A key for a mount row of the form, which keeps its fields while rows are added and removed. */
export const nextMountKey = () => ++mountKey

/** The draft of a container's settings, or for a new one the installation's defaults. */
export function specDraft(spec: Partial<ContainerSpec>, settings?: Settings): SpecDraft {
  return {
    resolution: spec.resolution || RESOLUTIONS[0],
    shm_mb: String(spec.shm_mb ?? 512),
    tmp_mb: String(spec.tmp_mb ?? 512),
    cpus: spec.cpus ? String(spec.cpus) : "",
    memory_mb: spec.memory_mb ? String(spec.memory_mb) : "",
    ...featureDraft(spec, settings),
    ports: formatPorts(spec.ports),
    groups: spec.groups ?? [],
    aliases: spec.aliases ?? [],
    mounts: (spec.mounts ?? []).map((mount) => ({ ...mount, key: nextMountKey() })),
    proxy: proxyDraft(spec, settings),
  }
}

/** The request body for the settings shared by creation and later changes. */
export function specBody(draft: SpecDraft) {
  const number = (value: string) => (value ? Number(value) : null)
  return {
    resolution: draft.resolution,
    shm_mb: number(draft.shm_mb),
    tmp_mb: number(draft.tmp_mb),
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

/** The features a draft turns on and its screen size, for a form's footer. */
export function specSummary(draft: SpecDraft) {
  const on = FLAGS.filter((flag) => draft[flag]).map(flagLabel)
  return [on.length ? formatList(on) : t("spec.noFeatures"), draft.resolution.replace("x", "×")]
}

/**
 * A switch with its icon and name on a tile the whole of which turns it, with what it does in a
 * tooltip. A disabled tile gets no hover or focus, so its reason belongs beside the tiles.
 */
export function SwitchTile({
  icon: Icon,
  label,
  description,
  checked,
  onCheckedChange,
  disabled,
  describedBy,
  className,
}: {
  icon: RemixiconComponentType
  label: string
  description?: string
  checked: boolean
  onCheckedChange: (checked: boolean) => void
  disabled?: boolean
  describedBy?: string
  className?: string
}) {
  const id = React.useId()
  const tile = (
    <label
      htmlFor={id}
      className={cn(
        "flex min-h-10 cursor-pointer items-center gap-2.5 rounded-xl bg-input/40 px-3 py-2 text-sm transition-colors hover:bg-input/70 has-disabled:cursor-not-allowed has-disabled:opacity-50 dark:bg-muted/50 dark:hover:bg-muted",
        className
      )}
    >
      <Icon
        aria-hidden
        className={cn("size-4 shrink-0", checked ? "text-foreground" : "text-muted-foreground")}
      />
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <Switch
        id={id}
        checked={checked}
        disabled={disabled}
        aria-describedby={describedBy}
        onCheckedChange={onCheckedChange}
      />
    </label>
  )
  if (!description || disabled) return tile
  return (
    <Tooltip>
      <TooltipTrigger asChild>{tile}</TooltipTrigger>
      <TooltipContent>{description}</TooltipContent>
    </Tooltip>
  )
}

/**
 * The features of a container as switches. `spec` is the container being changed, if any: a
 * feature the installation switched off cannot be turned on, but a container that has it can
 * still turn it off.
 */
export function FeatureFields({
  draft,
  onChange,
  settings,
  spec,
}: {
  draft: SpecDraft
  onChange: (draft: SpecDraft) => void
  settings?: Settings
  spec?: ContainerSpec
}) {
  const id = React.useId()
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
  const toggle = (flag: Flag, value: boolean) => {
    const patch: Partial<SpecDraft> = { [flag]: value }
    // The microphone travels over the audio connection.
    if (flag === "microphone" && value) patch.audio = true
    if (flag === "audio" && !value) patch.microphone = false
    onChange({ ...draft, ...patch })
  }

  return (
    <FieldSet className="gap-3">
      <FieldLegend variant="label" className="mb-0">
        {t("spec.features")}
      </FieldLegend>
      <div className="grid grid-cols-1 gap-2 min-[400px]:grid-cols-2">
        {FLAGS.map((flag) => (
          <SwitchTile
            key={flag}
            icon={FLAG_ICONS[flag]}
            label={flagLabel(flag)}
            description={flagDescription(flag)}
            checked={draft[flag]}
            disabled={blocked.includes(flag)}
            describedBy={blocked.includes(flag) ? `${id}-blocked` : undefined}
            onCheckedChange={(value) => toggle(flag, value)}
          />
        ))}
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
  )
}
