import type { MessageKey } from "@/locales/en"

import type { ConfigVariable, ContainerSpec, GpuReport, Settings, UpdateStatus } from "./types"

/** The sections of the settings page's configuration, in their order. */
export const SECTIONS = [
  "access",
  "mcp",
  "desktops",
  "features",
  "proxy",
  "gpu",
  "limits",
  "content",
  "manager",
  "notifications",
  "misc",
] as const
export type Section = (typeof SECTIONS)[number]

/** The section of each setting the manager lists, by its name in `.env`. */
export const SECTION_OF: Record<string, Section> = {
  AUTH_MODE: "access",
  ADMIN_USERNAME: "access",
  ADMIN_PASSWORD: "access",
  SECURE_COOKIE: "access",
  ALLOWED_HOSTS: "access",
  DESKTOP_HOST: "access",
  ROOT_MCP_ENABLED: "mcp",
  MCP_AUTH_MODE: "mcp",
  MCP_TOKEN: "mcp",
  DESKTOP_IMAGE: "desktops",
  DESKTOP_USER_NAMESPACES: "desktops",
  DESKTOP_PIDS_LIMIT: "desktops",
  AUDIO_ENABLED: "features",
  MICROPHONE_ENABLED: "features",
  INTERNET_ENABLED: "features",
  SUDO_ENABLED: "features",
  GAMEPAD_ENABLED: "features",
  VPN_ENABLED: "features",
  GPU_DEFAULT: "features",
  INTERNET_DEFAULT: "features",
  SUDO_DEFAULT: "features",
  AUDIO_DEFAULT: "features",
  MICROPHONE_DEFAULT: "features",
  GAMEPAD_DEFAULT: "features",
  VPN_DEFAULT: "features",
  IDLE_STOP_DEFAULT: "features",
  WSL_GPU: "gpu",
  NVIDIA_COMPUTE: "gpu",
  DRI_GPU: "gpu",
  GPU_ADAPTER: "gpu",
  UPLOAD_MAX_MB: "limits",
  IMAGE_ARCHIVE_MAX_MB: "limits",
  TEMPLATES_URL: "content",
  VOLUME_FILTER: "content",
  PROXY_URL: "proxy",
  PROXY_USERNAME: "proxy",
  PROXY_PASSWORD: "proxy",
  PROXY_BYPASS: "proxy",
  MANAGER_CONTAINER: "manager",
  METRICS_TOKEN: "manager",
  UPDATE_CHECK: "manager",
  NOTIFY_URL: "notifications",
  NOTIFY_EVENTS: "notifications",
  DASHBOARD_URL: "notifications",
}

/**
 * The settings by section, in the manager's order within each. A setting a newer manager added
 * and this page does not know yet is listed under `misc` rather than left out.
 */
export function sections(variables: ConfigVariable[]) {
  return SECTIONS.map((section) => ({
    section,
    variables: variables.filter((variable) => (SECTION_OF[variable.name] ?? "misc") === section),
  })).filter((entry) => entry.variables.length > 0)
}

/**
 * A section's settings as the page's rows, in the manager's order: a `<NAME>_ENABLED` and the
 * `<NAME>_DEFAULT` of the same section share one, where the first of them was, since one allows
 * a feature and the other gives it to a new container. Every other setting has a row of its own.
 */
export function settingRows(variables: ConfigVariable[]): ConfigVariable[][] {
  const byName = new Map(variables.map((variable) => [variable.name, variable]))
  const placed = new Set<string>()
  const rows: ConfigVariable[][] = []
  for (const variable of variables) {
    if (placed.has(variable.name)) continue
    const base = /^(.+)_(?:ENABLED|DEFAULT)$/.exec(variable.name)?.[1]
    const [enabled, fallback] = base
      ? [byName.get(`${base}_ENABLED`), byName.get(`${base}_DEFAULT`)]
      : []
    const row = enabled && fallback ? [enabled, fallback] : [variable]
    for (const entry of row) placed.add(entry.name)
    rows.push(row)
  }
  return rows
}

/** The switches a setting needs beside its own: the microphone travels over the audio connection. */
const NEEDS: Record<string, string> = { MICROPHONE_ENABLED: "AUDIO_ENABLED" }

/**
 * The settings that do nothing as the switches stand, each with the switch that is off: a
 * `<NAME>_DEFAULT` gives a new container only what its `<NAME>_ENABLED` allows, as
 * `Config::feature_defaults` reads them, and the microphone needs sound. `on` reads a switch as
 * the page shows it.
 */
export function idleSettings(
  variables: ConfigVariable[],
  on: (name: string) => boolean
): Record<string, string> {
  const names = new Set(variables.map((variable) => variable.name))
  const off = (name: string): string | undefined => {
    const base = /^(.+)_DEFAULT$/.exec(name)?.[1]
    const needed = NEEDS[name] ?? (base && `${base}_ENABLED`)
    if (!needed || !names.has(needed)) return undefined
    return off(needed) ?? (on(needed) ? undefined : needed)
  }
  const idle: Record<string, string> = {}
  for (const { name } of variables) {
    const by = off(name)
    if (by) idle[name] = by
  }
  return idle
}

/** How a setting's value reads: hidden, not set, or the text `.env` would hold. */
export function shownValue(
  variable: ConfigVariable
): { kind: "secret"; set: boolean } | { kind: "empty" } | { kind: "text"; text: string } {
  if (variable.secret) return { kind: "secret", set: variable.set }
  if (variable.value === null || variable.value === "") return { kind: "empty" }
  return { kind: "text", text: String(variable.value) }
}

/** A choice of a setting with a few meaningful values; `label` is a catalog key, or the value is shown. */
export type Choice = { value: string; label?: MessageKey }

/** How the Settings page edits a setting the manager marks editable. */
export type Field =
  | { kind: "switch" }
  | { kind: "number"; min: number; max: number }
  | { kind: "text"; placeholder?: string }
  /** A few values, and `other` to type one, with `placeholder` in its field. */
  | { kind: "choice"; choices: Choice[]; placeholder?: string }

/** The editable settings' controls. One a newer manager adds and this page does not know is text. */
export const FIELDS: Record<string, Field> = {
  SECURE_COOKIE: { kind: "switch" },
  ALLOWED_HOSTS: { kind: "text", placeholder: "desktop.lan,nas.lan" },
  DESKTOP_IMAGE: { kind: "text", placeholder: "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04" },
  DESKTOP_USER_NAMESPACES: { kind: "switch" },
  // 0 is no limit; the manager refuses one below 1024, which leaves desktops that cannot start.
  DESKTOP_PIDS_LIMIT: { kind: "number", min: 0, max: 4194304 },
  WSL_GPU: { kind: "switch" },
  NVIDIA_COMPUTE: { kind: "switch" },
  GPU_ADAPTER: {
    kind: "choice",
    choices: [
      { value: "", label: "settings.choice.automatic" },
      { value: "NVIDIA" },
      { value: "AMD" },
      { value: "Intel" },
    ],
    placeholder: "GeForce RTX 4090",
  },
  AUDIO_ENABLED: { kind: "switch" },
  MICROPHONE_ENABLED: { kind: "switch" },
  INTERNET_ENABLED: { kind: "switch" },
  SUDO_ENABLED: { kind: "switch" },
  GAMEPAD_ENABLED: { kind: "switch" },
  VPN_ENABLED: { kind: "switch" },
  GPU_DEFAULT: { kind: "switch" },
  INTERNET_DEFAULT: { kind: "switch" },
  SUDO_DEFAULT: { kind: "switch" },
  AUDIO_DEFAULT: { kind: "switch" },
  MICROPHONE_DEFAULT: { kind: "switch" },
  GAMEPAD_DEFAULT: { kind: "switch" },
  VPN_DEFAULT: { kind: "switch" },
  // Minutes, 0 for never; the manager takes up to a week.
  IDLE_STOP_DEFAULT: { kind: "number", min: 0, max: 10080 },
  UPLOAD_MAX_MB: { kind: "number", min: 1, max: 65536 },
  IMAGE_ARCHIVE_MAX_MB: { kind: "number", min: 1, max: 1048576 },
  TEMPLATES_URL: {
    kind: "choice",
    choices: [
      { value: "", label: "settings.choice.wiki" },
      { value: "none", label: "settings.choice.off" },
    ],
    placeholder: "https://example.com/templates/index.json",
  },
  VOLUME_FILTER: {
    kind: "choice",
    choices: [
      { value: "", label: "settings.choice.ownLabel" },
      { value: "none", label: "settings.choice.everyVolume" },
    ],
    placeholder: "team=desktops",
  },
  PROXY_URL: { kind: "text", placeholder: "http://proxy.example:3128" },
  PROXY_BYPASS: { kind: "text", placeholder: "intranet.example,10.0.0.0/8" },
  UPDATE_CHECK: { kind: "switch" },
  NOTIFY_EVENTS: {
    kind: "choice",
    choices: [
      { value: "", label: "settings.choice.humanRequests" },
      { value: "none", label: "settings.choice.off" },
    ],
    placeholder: "human_request,job_failed,update_available",
  },
  DASHBOARD_URL: { kind: "text", placeholder: "https://thoric.lan:8443" },
}

export const fieldOf = (name: string): Field => FIELDS[name] ?? { kind: "text" }

/** The kinds of notice `NOTIFY_EVENTS` names, each with its label. */
export const NOTIFY_EVENTS: Record<string, MessageKey> = {
  human_request: "notify.event.human_request",
  job_failed: "notify.event.job_failed",
  update_available: "notify.event.update_available",
}

/**
 * The kinds of notice `NOTIFY_EVENTS` sends as the manager reads it, or `null` for none. A kind
 * this page does not know yet keeps its name, without a label.
 */
export function notifyEvents(value: string): { name: string; label?: MessageKey }[] | null {
  const names = value
    .split(",")
    .map((name) => name.trim())
    .filter((name) => name && name !== "none")
  if (!names.length) return null
  return names.map((name) =>
    NOTIFY_EVENTS[name] ? { name, label: NOTIFY_EVENTS[name] } : { name }
  )
}

/** Which of a choice's values a text is, or -1 for a text of the user's own. */
export const choiceIndex = (field: Extract<Field, { kind: "choice" }>, text: string) =>
  field.choices.findIndex((choice) => choice.value === text)

/** Whether a switch's text is on, as the manager reads it. */
export const isOn = (text: string) => text === "true" || text === "1"

/**
 * The text an editable setting starts from: the one its environment gave, or for a switch, a
 * number or a text left unset, the value the manager made of it, since the control has to show one.
 */
export function initialText(variable: ConfigVariable): string {
  if (variable.secret) return ""
  const text = variable.text ?? ""
  if (text !== "") return text
  const field = fieldOf(variable.name)
  if (field.kind === "switch") return variable.value === true ? "true" : "false"
  if (field.kind === "number" && typeof variable.value === "number") return String(variable.value)
  // A text field's value reads as its text; a choice's empty text is one of its choices.
  if (field.kind === "text" && typeof variable.value === "string") return variable.value
  return text
}

/**
 * What is wrong with a text before the manager is asked: the line `.env` keeps as it is, as the
 * manager and `thoric apply-settings` check it, and a number's range. The manager says the
 * rest when it checks the change.
 */
export function textProblem(
  name: string,
  text: string
): { key: MessageKey; values?: Record<string, string | number> } | null {
  const field = fieldOf(name)
  if (field.kind === "number") {
    const number = Number(text)
    if (!/^[0-9]+$/.test(text) || number < field.min || number > field.max)
      return { key: "settings.edit.problem.number", values: { min: field.min, max: field.max } }
    return null
  }
  if (
    text.length > 2048 ||
    text.trim() !== text ||
    !/^[ -~]*$/.test(text) ||
    /["'`$#\\]/.test(text)
  )
    return { key: "settings.edit.problem.text" }
  return null
}

/** The changes a draft makes: each setting whose text differs from the one it started from. */
export function changesOf(variables: ConfigVariable[], draft: Record<string, string>) {
  const changes: Record<string, string> = {}
  for (const variable of variables) {
    if (variable.secret || !variable.editable || !(variable.name in draft)) continue
    if (draft[variable.name] !== initialText(variable))
      changes[variable.name] = draft[variable.name]
  }
  return changes
}

/** Where this installation stands against the newest release. */
export function releaseState(
  update: Pick<UpdateStatus, "enabled" | "job_id" | "available" | "newest" | "error">
): "off" | "updating" | "available" | "current" | "failed" | "unchecked" {
  if (!update.enabled) return "off"
  if (update.job_id) return "updating"
  if (update.available) return "available"
  // A failed look keeps the release seen before, which still tells whether this one is current.
  if (update.newest) return "current"
  return update.error ? "failed" : "unchecked"
}

/** The features of the create and settings forms. */
export const FEATURES = [
  "gpu",
  "internet",
  "sudo",
  "audio",
  "microphone",
  "gamepad",
  "vpn",
] as const
export type Feature = (typeof FEATURES)[number]

/**
 * A container's features, or a new container's from the installation's defaults (`GPU_DEFAULT`
 * and the others), which never include one the installation switched off. A manager from before
 * the defaults sends none, and the form starts as it did then.
 */
export function featureDraft(
  spec: Partial<ContainerSpec>,
  settings?: Settings
): Record<Feature, boolean> {
  const defaults = settings?.defaults
  const audio = spec.audio ?? defaults?.audio ?? false
  return {
    gpu: spec.gpu ?? defaults?.gpu ?? settings?.gpu_enabled !== false,
    internet: spec.internet ?? defaults?.internet ?? true,
    sudo: spec.sudo ?? defaults?.sudo ?? true,
    audio,
    // The microphone travels over the audio connection.
    microphone: spec.microphone ?? (audio && (defaults?.microphone ?? false)),
    gamepad: spec.gamepad ?? defaults?.gamepad ?? false,
    vpn: spec.vpn ?? defaults?.vpn ?? false,
  }
}

/**
 * What the Settings page says about the host's GPU: the routes GPU desktops get, and the routes
 * their settings allow that are worth a word. With no route that is every one of them, which says
 * why desktops have no GPU; otherwise only an NVIDIA GPU that Docker cannot give them. A route the
 * host simply lacks, such as `/dev/dri` in WSL, is what the default settings expect.
 */
export function gpuNotes(report: GpuReport) {
  return {
    routes: report.routes,
    unused: report.routes.length
      ? report.unused.filter((entry) => entry.reason === "no_toolkit")
      : report.unused,
  }
}
