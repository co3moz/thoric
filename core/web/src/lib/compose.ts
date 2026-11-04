import type {
  ContainerSetup,
  ImageItem,
  ParameterDisplay,
  ParameterOption,
  ParameterType,
  Compose,
  ComposeParameter,
  ComposeStep,
  Volume,
} from "./types"
import { t } from "./i18n"

/** The manager's rule for a parameter name, as an input pattern. */
export const PARAMETER_NAME = "[A-Z][A-Z0-9_]{0,63}"

export const DEFAULT_SETUP = `#!/usr/bin/env bash
set -euo pipefail
# Each parameter is an environment variable here, such as "$UNITY_EMAIL".
`

/** How the create form can show each type, the default first; the manager accepts these. */
export const DISPLAYS: Record<ParameterType, ParameterDisplay[]> = {
  text: ["input", "multiline"],
  number: ["input", "slider"],
  boolean: ["toggle", "checkbox"],
  choice: ["select", "radio"],
  choices: ["checkboxes"],
  volume: ["select"],
  folder: ["input"],
}

export const typeLabel = (type: ParameterType) => t(`compose.type.${type}`)

export const displayLabel = (display: ParameterDisplay) => t(`compose.display.${display}`)

export const displayOf = (parameter: Pick<ComposeParameter, "type" | "display">) =>
  parameter.display && DISPLAYS[parameter.type].includes(parameter.display)
    ? parameter.display
    : DISPLAYS[parameter.type][0]

const hasOptions = (type: ParameterType) => type === "choice" || type === "choices"

type WithCompose = Pick<ImageItem, "image_id" | "reference" | "compose">

/**
 * The record whose compose a new container from `value`, an image ID or reference, asks for.
 * The manager takes it from the first record of that image that has one; lists come newest
 * first. A build on that image keeps the same compose before its own.
 */
export function composeImage<R extends WithCompose>(records: R[], value?: string) {
  if (!value) return undefined
  return records.find(
    (record) => record.compose && (record.image_id === value || record.reference === value)
  )
}

/** The compose a new container from `value`, an image ID or reference, asks for. */
export const composeFor = (records: WithCompose[], value?: string) =>
  composeImage(records, value)?.compose ?? undefined

type StepFields = Pick<ComposeStep, "id" | "name" | "parameters">

/** A compose's steps: its one step, or the several it lists. */
export const composeSteps = <S extends StepFields>(compose: S | { steps: S[] }): S[] =>
  "steps" in compose ? compose.steps : [compose]

/** What the forms call a step. */
export const stepTitle = (step: Pick<ComposeStep, "id" | "name">) => step.name || step.id || ""

/**
 * What the steps of an image's compose are called once another image keeps them: the one step
 * of a compose that names none goes by the image's name, as the manager names it.
 */
export const imageSteps = (image: Pick<ImageItem, "name" | "compose">) =>
  image.compose ? composeSteps(image.compose).map((step) => stepTitle(step) || image.name) : []

/**
 * Every parameter of a compose with its step and the name its value goes by: the parameter's
 * own name, or `<step>.<NAME>` in a compose of several steps, where two steps may ask for one.
 */
export function composeEntries<S extends StepFields>(compose: S | { steps: S[] }) {
  const steps = composeSteps(compose)
  return steps.flatMap((step) =>
    step.parameters.map((parameter) => ({
      step,
      parameter,
      key: steps.length > 1 ? `${step.id}.${parameter.name}` : parameter.name,
    }))
  )
}

/** The values a create request sends: one for every parameter, its default when left alone. */
export function parameterBody(
  compose: StepFields | { steps: StepFields[] } | null | undefined,
  values: Record<string, string>
) {
  return Object.fromEntries(
    (compose ? composeEntries(compose) : []).map(({ parameter, key }) => [
      key,
      values[key] ?? parameter.default,
    ])
  )
}

/**
 * Whether a required parameter still lacks its value, as the manager refuses it: a switch that is
 * off, and an empty choice, volume, number or text. An empty volume is filled when the parameter
 * makes a new one for the container.
 */
export function valueMissing(
  parameter: Pick<ComposeParameter, "type" | "required" | "new_volume">,
  value: string
) {
  if (!parameter.required) return false
  if (parameter.type === "boolean") return value !== "true"
  if (parameter.type === "volume")
    return value === WITHOUT_VOLUME || volumeSelection(parameter, value, true) === ""
  return value === ""
}

/** How many required values each step of a compose still lacks, in the order of its steps. */
export function missingValues(
  compose: StepFields | { steps: StepFields[] },
  values: Record<string, string>
) {
  const steps = composeSteps(compose)
  const missing = steps.map(() => 0)
  for (const { step, parameter, key } of composeEntries(compose))
    if (valueMissing(parameter, values[key] ?? parameter.default)) missing[steps.indexOf(step)] += 1
  return missing
}

/** A volume a compose mounts: a named one, or `null` for the one the container gets of its own. */
export type SetupVolume = { volume: string | null; target: string; step?: string }

/**
 * The volumes the values of a compose mount, where its volume parameters put them, with the step
 * that asks when there are several. A parameter left without a volume mounts none.
 */
export function setupVolumes(
  compose: StepFields | { steps: StepFields[] },
  values: Record<string, string>
): SetupVolume[] {
  const several = composeSteps(compose).length > 1
  return composeEntries(compose).flatMap(({ step, parameter, key }) => {
    if (parameter.type !== "volume" || !parameter.path) return []
    const value = values[key] ?? parameter.default
    const selected = volumeSelection(parameter, value, parameter.required)
    if (selected === NO_VOLUME || selected === "") return []
    return [
      {
        volume: selected === NEW_VOLUME ? null : value,
        target: parameter.path,
        ...(several ? { step: stepTitle(step) } : {}),
      },
    ]
  })
}

type Inheriting = Pick<ImageItem, "image_id" | "parent_image_id" | "compose" | "name"> &
  Partial<Pick<ImageItem, "template_ids">>

/**
 * The steps an image's compose keeps from the saved image it was built on, by their ids, with
 * that image's name, as the manager joins them: the base's steps come first, and the one step of
 * a base compose that names none goes by the template the base was built from, or `base`. An
 * image whose steps all came from its parent, such as a snapshot, names none.
 */
export function inheritedSteps(image: Inheriting | undefined, records: Inheriting[]) {
  if (!image?.compose || !("steps" in image.compose) || !image.parent_image_id) return null
  const parent = records.find(
    (record) => record.compose && record.image_id === image.parent_image_id
  )
  if (!parent?.compose) return null
  const templates = parent.template_ids ?? []
  const base = composeSteps(parent.compose)
  const ids = new Set(
    base.map((step) =>
      step.id ? step.id : base.length === 1 && templates.length === 1 ? templates[0] : "base"
    )
  )
  const own = image.compose.steps.filter((step) => !ids.has(step.id ?? ""))
  if (!own.length || own.length === image.compose.steps.length) return null
  return { name: parent.name, ids }
}

/** The options picked in a `choices` value, which holds one per line. */
export const pickedChoices = (value: string) => (value ? value.split("\n") : [])

/** A `choices` value with `option` turned on or off, its lines in the options' order. */
export function toggleChoice(
  value: string,
  option: string,
  on: boolean,
  options: ParameterOption[]
) {
  const picked = new Set(pickedChoices(value))
  if (on) picked.add(option)
  else picked.delete(option)
  return options
    .map((item) => item.value)
    .filter((item) => picked.has(item))
    .join("\n")
}

/** The manager's rule for a volume name, as an input pattern and a check. */
export const VOLUME_NAME = "[a-zA-Z0-9][a-zA-Z0-9_.\\-]+"
export const volumeNameValid = (name: string) =>
  name.length <= 128 && new RegExp(`^${VOLUME_NAME}$`).test(name)

/** The volumes a volume parameter offers: every named volume, and its value if that is gone. */
export function volumeChoices(volumes: Pick<Volume, "Name">[], value: string) {
  const names = volumes.map((volume) => volume.Name).sort((a, b) => a.localeCompare(b))
  return value && !names.includes(value) ? [value, ...names] : names
}

// Values never contain NUL, so these cannot collide with a volume's name.
/** The select item for no volume. */
export const NO_VOLUME = "\u0000none"
/**
 * The select item for a volume the creation job makes for the container. The editor holds a
 * default of that kind as this value too; the create form sends it as an empty value.
 */
export const NEW_VOLUME = "\u0000new"
/**
 * The value that asks the manager for no volume where an empty one makes a new volume. No
 * volume's name starts with a dash.
 */
export const WITHOUT_VOLUME = "-"

/**
 * What a volume parameter's select offers before the volumes. The form offers no volume unless
 * one is required, and a new one only when the parameter makes it; the editor of `defaults`
 * offers both.
 */
export function volumeExtras(
  parameter: Pick<ComposeParameter, "new_volume">,
  { required, defaults }: { required: boolean; defaults: boolean }
) {
  return {
    none: defaults || !required,
    fresh: defaults || !!parameter.new_volume,
  }
}

/**
 * The select item for `value`. An empty value is the new volume when the parameter makes one,
 * otherwise no volume, or nothing chosen yet for a required parameter.
 */
export function volumeSelection(
  parameter: Pick<ComposeParameter, "new_volume">,
  value: string,
  required: boolean
) {
  if (value === WITHOUT_VOLUME) return NO_VOLUME
  if (value) return value
  if (parameter.new_volume) return NEW_VOLUME
  return required ? "" : NO_VOLUME
}

/**
 * The form's value for no volume: empty, unless an empty value makes a new volume. The editor of
 * `defaults` keeps it empty, since a default of none makes no new volume.
 */
export const noVolumeValue = (
  parameter: Pick<ComposeParameter, "new_volume">,
  defaults: boolean
) => (parameter.new_volume && !defaults ? WITHOUT_VOLUME : "")

/** The editor's value for a volume parameter's default: a volume, `""` for none, or NEW_VOLUME. */
export const volumeDefault = (draft: Pick<ParameterDraft, "default" | "new_volume">) =>
  draft.new_volume ? NEW_VOLUME : draft.default

/** A draft with the default the editor's volume select picked; a new volume has no default. */
export function withVolumeDefault(draft: ParameterDraft, value: string): ParameterDraft {
  const fresh = value === NEW_VOLUME
  return { ...draft, new_volume: fresh, default: fresh ? "" : value }
}

/** A name for a new volume of `parameter`, such as `unity-home` for UNITY_HOME, not yet taken. */
export function newVolumeName(
  parameter: Pick<ComposeParameter, "name">,
  volumes: Pick<Volume, "Name">[]
) {
  // Parameter names start with a letter, so the name does too.
  const base = parameter.name.toLowerCase().replace(/_+/g, "-").replace(/-+$/, "")
  const taken = new Set(volumes.map((volume) => volume.Name))
  let name = base.length < 2 ? `${base}-volume` : base
  for (let number = 2; taken.has(name); number++) name = `${base}-${number}`
  return name
}

/**
 * A text parameter's pattern as this browser checks it, matching a whole value as the manager
 * does, or `null` when the browser cannot read it. The manager reads Rust's syntax, which this
 * only approximates, so its answer is the one that counts either way.
 */
export function browserPattern(pattern: string) {
  try {
    // On its own first, so a pattern such as `a)|(b` cannot close the group around it.
    new RegExp(pattern, "u")
    return new RegExp(`^(?:${pattern})$`, "u")
  } catch {
    return null
  }
}

/**
 * Whether a text value has its pattern's format, or `undefined` when only the manager can tell.
 * An empty value is left to `required`.
 */
export const patternMatches = (pattern: string | undefined, value: string) =>
  !pattern || !value || browserPattern(pattern)?.test(value)

/** A slider position as the script reads it, with no more decimals than its step has. */
export function sliderValue(position: number, step?: number | null) {
  const decimals = String(step ?? 1).split(".")[1]?.length ?? 0
  return String(Number(position.toFixed(decimals)))
}

export type OptionDraft = ParameterOption & { key: number }
/** A parameter as the editor holds it: every field of every type, numbers as typed. */
export type ParameterDraft = {
  key: number
  name: string
  label: string
  description: string
  type: ParameterType
  display: ParameterDisplay
  required: boolean
  secret: boolean
  default: string
  options: OptionDraft[]
  allow_custom: boolean
  min: string
  max: string
  step: string
  path: string
  pattern: string
  pattern_message: string
  new_volume: boolean
}
/** A step as the editor holds it; `id` and `name` stay as the compose named them. */
export type StepDraft = {
  key: number
  id: string
  name: string
  parameters: ParameterDraft[]
  script: string
  root: boolean
  timeout_seconds: string
}
export type ComposeDraft = { steps: StepDraft[] }

let draftKey = 0

export const newOption = (): OptionDraft => ({ key: ++draftKey, value: "", label: "" })

export const newParameter = (): ParameterDraft => ({
  key: ++draftKey,
  name: "",
  label: "",
  description: "",
  type: "text",
  display: "input",
  required: false,
  secret: false,
  default: "",
  options: [],
  allow_custom: false,
  min: "",
  max: "",
  step: "",
  path: "",
  pattern: "",
  pattern_message: "",
  new_volume: false,
})

const bound = (value?: number | null) => (value === undefined || value === null ? "" : `${value}`)

function parameterDraft(parameter: ComposeParameter): ParameterDraft {
  return {
    ...newParameter(),
    name: parameter.name,
    label: parameter.label,
    description: parameter.description,
    type: parameter.type,
    display: displayOf(parameter),
    required: parameter.required,
    secret: parameter.secret,
    default: parameter.default,
    options: (parameter.options ?? []).map((option) => ({ ...option, key: ++draftKey })),
    allow_custom: parameter.allow_custom ?? false,
    min: bound(parameter.min),
    max: bound(parameter.max),
    step: bound(parameter.step),
    path: parameter.path ?? "",
    pattern: parameter.pattern ?? "",
    pattern_message: parameter.pattern_message ?? "",
    new_volume: parameter.new_volume ?? false,
  }
}

/**
 * A draft with another type. The default and the fields only the old type had are cleared, since
 * the new type would refuse them; options stay when both types have them.
 */
export function withType(draft: ParameterDraft, type: ParameterType): ParameterDraft {
  return {
    ...draft,
    type,
    display: DISPLAYS[type][0],
    default: "",
    secret: type === "text" && draft.secret,
    allow_custom: type === "choice" && draft.allow_custom,
    new_volume: false,
    options: hasOptions(type) && !draft.options.length ? [newOption()] : draft.options,
  }
}

/** A draft with new options. A default that picks an option no longer offered is dropped. */
export function withOptions(draft: ParameterDraft, options: OptionDraft[]): ParameterDraft {
  const offered = new Set(options.map((option) => option.value.trim()))
  const kept =
    draft.type === "choices"
      ? pickedChoices(draft.default)
          .filter((line) => offered.has(line))
          .join("\n")
      : draft.allow_custom || offered.has(draft.default)
        ? draft.default
        : ""
  return { ...draft, options, default: kept }
}

/** The parameter a draft describes, with only the fields its type has. */
export function draftParameter(draft: ParameterDraft): ComposeParameter {
  const secret = draft.type === "text" && draft.secret
  const number = (text: string) => (text.trim() ? { value: Number(text) } : undefined)
  const range = Object.fromEntries(
    (["min", "max", "step"] as const).flatMap((field) => {
      const value = number(draft[field])
      return draft.type === "number" && value ? [[field, value.value]] : []
    })
  )
  return {
    name: draft.name.trim(),
    label: draft.label.trim(),
    description: draft.description.trim(),
    type: draft.type,
    ...(draft.display !== DISPLAYS[draft.type][0] ? { display: draft.display } : {}),
    required: draft.required,
    secret,
    // Image lists show defaults, so a secret has none.
    default: secret ? "" : draft.default,
    ...(hasOptions(draft.type)
      ? {
          options: draft.options.map((option) => ({
            value: option.value.trim(),
            label: option.label.trim(),
          })),
        }
      : {}),
    ...(draft.type === "choice" && draft.allow_custom ? { allow_custom: true } : {}),
    ...range,
    ...(draft.type === "volume" ? { path: draft.path.trim() } : {}),
    ...(draft.type === "volume" && draft.new_volume ? { new_volume: true } : {}),
    // Spaces can be part of a pattern, and a message without one is never shown.
    ...(draft.type === "text" && draft.pattern
      ? {
          pattern: draft.pattern,
          ...(draft.pattern_message.trim()
            ? { pattern_message: draft.pattern_message.trim() }
            : {}),
        }
      : {}),
  }
}

function stepDraft(step?: ComposeStep): StepDraft {
  return {
    key: ++draftKey,
    id: step?.id ?? "",
    name: step?.name ?? "",
    parameters: (step?.parameters ?? []).map(parameterDraft),
    script: step?.script ?? DEFAULT_SETUP,
    root: step?.root ?? false,
    timeout_seconds: String(step?.timeout_seconds ?? 600),
  }
}

export function composeDraft(compose?: Compose | null): ComposeDraft {
  return { steps: compose ? composeSteps(compose).map(stepDraft) : [stepDraft()] }
}

function draftStep(draft: StepDraft): ComposeStep {
  return {
    ...(draft.id ? { id: draft.id } : {}),
    ...(draft.name ? { name: draft.name } : {}),
    parameters: draft.parameters.map(draftParameter),
    script: draft.script,
    root: draft.root,
    timeout_seconds: Number(draft.timeout_seconds),
  }
}

/** The compose a draft describes: one step as composes were before steps, or several. */
export function composeBody(draft: ComposeDraft): Compose {
  const steps = draft.steps.map(draftStep)
  const [only] = steps
  return steps.length === 1 && !only.id && !only.name ? only : { steps }
}

/** Steps put in the order of `ids`, such as the parts of a build; others keep their place after. */
export function orderSteps<S extends Pick<ComposeStep, "id">>(steps: S[], ids: string[]) {
  const rank = (step: S) => {
    const index = ids.indexOf(step.id ?? "")
    return index < 0 ? ids.length : index
  }
  return steps
    .map((step, index) => ({ step, index }))
    .sort((a, b) => rank(a.step) - rank(b.step) || a.index - b.index)
    .map(({ step }) => step)
}

const TYPES = Object.keys(DISPLAYS) as ParameterType[]

function readParameter(value: unknown): ComposeParameter {
  const parameter = (value ?? {}) as Record<string, unknown>
  const { name } = parameter
  if (typeof name !== "string" || !name) throw new Error(t("compose.read.noName"))
  const wrong = () => new Error(t("compose.read.shape", { name }))
  const text = (key: string) => {
    const field = parameter[key] ?? ""
    if (typeof field !== "string") throw wrong()
    return field
  }
  const flag = (key: string) => {
    const field = parameter[key] ?? false
    if (typeof field !== "boolean") throw wrong()
    return field
  }
  const type = (parameter.type ?? "text") as ParameterType
  if (!TYPES.includes(type)) throw new Error(t("compose.read.type", { name }))
  const display = parameter.display ?? undefined
  if (display !== undefined && !DISPLAYS[type].includes(display as ParameterDisplay))
    throw new Error(t("compose.read.display", { name }))
  const options = parameter.options ?? []
  if (!Array.isArray(options)) throw wrong()
  const read: ComposeParameter = {
    name,
    label: text("label"),
    description: text("description"),
    type,
    ...(display ? { display: display as ParameterDisplay } : {}),
    required: flag("required"),
    secret: flag("secret"),
    default: text("default"),
    ...(options.length
      ? {
          options: options.map((option) => {
            const { value, label = "" } = (option ?? {}) as Record<string, unknown>
            if (typeof value !== "string" || typeof label !== "string") throw wrong()
            return { value, label }
          }),
        }
      : {}),
    ...(flag("allow_custom") ? { allow_custom: true } : {}),
    ...(text("path") ? { path: text("path") } : {}),
    ...(text("pattern") ? { pattern: text("pattern") } : {}),
    ...(text("pattern_message") ? { pattern_message: text("pattern_message") } : {}),
    ...(flag("new_volume") ? { new_volume: true } : {}),
  }
  for (const field of ["min", "max", "step"] as const) {
    const bound = parameter[field]
    if (bound === undefined || bound === null) continue
    if (typeof bound !== "number") throw wrong()
    read[field] = bound
  }
  return read
}

function readStep(value: unknown): ComposeStep {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error(t("compose.read.object"))
  const {
    id = "",
    name = "",
    parameters = [],
    script,
    root = false,
    timeout_seconds = 600,
  } = value as Record<string, unknown>
  if (typeof id !== "string" || typeof name !== "string") throw new Error(t("compose.read.step"))
  if (typeof script !== "string" || !script.trim()) throw new Error(t("compose.read.script"))
  if (!Array.isArray(parameters)) throw new Error(t("compose.read.parameters"))
  if (typeof root !== "boolean") throw new Error(t("compose.read.root"))
  if (typeof timeout_seconds !== "number") throw new Error(t("compose.read.timeout"))
  return {
    ...(id ? { id } : {}),
    ...(name ? { name } : {}),
    parameters: parameters.map(readParameter),
    script,
    root,
    timeout_seconds,
  }
}

/** A pasted configuration's compose, or the reason it cannot be one. */
export function readCompose(value: unknown): Compose {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error(t("compose.read.object"))
  if (!("steps" in value)) return readStep(value)
  const { steps } = value as { steps: unknown }
  if (!Array.isArray(steps) || !steps.length) throw new Error(t("compose.read.steps"))
  return { steps: steps.map(readStep) }
}

export const setupLabel = (setup: Pick<ContainerSetup, "status">) =>
  t(`compose.setup.${setup.status}`)

type SetupStep = NonNullable<ContainerSetup["steps"]>[number]

/**
 * A container's setup values by the step that asked for them, each by the name its step's script
 * reads; a compose of one step has one group without a step.
 */
export function setupGroups(
  setup: Pick<ContainerSetup, "parameters" | "secrets" | "steps">
): { step?: SetupStep; values: [string, string][]; secrets: string[] }[] {
  const steps = setup.steps ?? []
  if (!steps.length) return [{ values: Object.entries(setup.parameters), secrets: setup.secrets }]
  // Step ids hold no dot, so `<step>.` names exactly that step's values.
  const own = (step: SetupStep) => (name: string) =>
    name.startsWith(`${step.id}.`) ? [name.slice(step.id.length + 1)] : []
  return steps.map((step) => ({
    step,
    values: Object.entries(setup.parameters).flatMap(([name, value]) =>
      own(step)(name).map((short): [string, string] => [short, value])
    ),
    secrets: setup.secrets.flatMap(own(step)),
  }))
}
