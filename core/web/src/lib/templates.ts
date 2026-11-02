// The templates page: ready-made images listed on the project's wiki, or wherever TEMPLATES_URL
// points. A template is a build configuration as Export copies it; the list beside it names each
// one with a description and tags, and `<id>.png` next to it is its icon.
//
//   index.json   {"templates": [{"id": "unity", "name": "Unity Hub", "description": "…",
//                                "tags": ["games"], "requires": "0.5.0"},
//                               {"id": "node", "name": "Node.js", "kind": "environment",
//                                "architectures": ["amd64", "arm64"]}]}
//   unity.json   the build configuration
//   unity.png    the icon, at most 256 by 256 pixels
//
// Nothing checks a template before the list serves it, so every entry is checked here: one that
// cannot be offered shows why instead of breaking the page.
import { readBuildConfig, TEMPLATE_ID, type ImportedBuild } from "./build-config"
import { composeSteps } from "./compose"
import { iconFromBlob } from "./icons"
import { formatList, t } from "./i18n"
import type { ComposeStep, DraftPart, ServerDeclaration } from "./types"

/**
 * What a template brings: an application for the desktop, or an environment, such as a language
 * toolchain, that is meant to be merged with an application. The page shows them apart, in this
 * order.
 */
export const TEMPLATE_KINDS = ["application", "environment"] as const
export type TemplateKind = (typeof TEMPLATE_KINDS)[number]

export type Template = {
  id: string
  name: string
  description: string
  tags: string[]
  /** `application` when the list names none. */
  kind: TemplateKind
  /** The oldest Thoric release that can build it, such as `0.5.0`. */
  requires?: string
  /**
   * The architectures of the Docker hosts it builds on, such as `amd64` and `arm64`, as Docker
   * names them. `DEFAULT_ARCHITECTURES` when the list names none.
   */
  architectures?: string[]
  /** The names of the desktop MCP servers an image built from it declares. */
  mcp?: string[]
}

/**
 * What a template builds on when its entry names nothing: every template written before the field
 * was tried on x86_64 only, and most download programs made for it.
 */
export const DEFAULT_ARCHITECTURES = ["amd64"]

/** A list entry the page cannot offer, and why. */
export type BrokenTemplate = { id: string; reason: string }

export type TemplateList = { templates: Template[]; broken: BrokenTemplate[] }

const VERSION = /^\d+\.\d+\.\d+$/
const TAG_LIMIT = 8
const SERVER_NAME = /^[a-z0-9][a-z0-9-]{0,31}$/
const ARCHITECTURE_LIMIT = 8
// Docker's names, as `docker info` reports them to the manager. The kernel's names for the two
// common ones would never match a host, so they are refused rather than leaving the template
// unoffered without a word.
const ARCHITECTURE = /^[a-z0-9]{1,16}$/
const KERNEL_NAMES = new Set(["x86_64", "aarch64"])

/** The architecture names of an entry, or throws when they are not a short list of Docker's. */
function readArchitectures(value: unknown) {
  if (value === undefined) return undefined
  if (
    !Array.isArray(value) ||
    !value.length ||
    value.length > ARCHITECTURE_LIMIT ||
    value.some(
      (name) => typeof name !== "string" || !ARCHITECTURE.test(name) || KERNEL_NAMES.has(name)
    )
  )
    throw new Error(t("templates.broken.architectures", { count: ARCHITECTURE_LIMIT }))
  return [...new Set(value as string[])]
}

/** Reads one list entry, or throws why it cannot be offered. */
function readTemplate(value: unknown): Template {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error(t("templates.broken.entry"))
  const {
    id,
    name,
    description = "",
    tags = [],
    kind = "application",
    requires,
    architectures,
    mcp = [],
  } = value as Record<string, unknown>
  if (typeof id !== "string" || !TEMPLATE_ID.test(id)) throw new Error(t("templates.broken.id"))
  if (typeof name !== "string" || !name.trim() || [...name].length > 100)
    throw new Error(t("templates.broken.name"))
  if (typeof description !== "string" || [...description].length > 500)
    throw new Error(t("templates.broken.description"))
  if (
    !Array.isArray(tags) ||
    tags.length > TAG_LIMIT ||
    tags.some((tag) => typeof tag !== "string" || !tag.trim() || [...tag].length > 32)
  )
    throw new Error(t("templates.broken.tags", { count: TAG_LIMIT }))
  // A kind this page does not know would land in the wrong section, so the entry says so.
  if (!TEMPLATE_KINDS.includes(kind as TemplateKind)) throw new Error(t("templates.broken.kind"))
  if (requires !== undefined && (typeof requires !== "string" || !VERSION.test(requires)))
    throw new Error(t("templates.broken.requires"))
  const platforms = readArchitectures(architectures)
  // Only names, for the card; the declarations come with the template's configuration.
  const servers = Array.isArray(mcp)
    ? mcp.filter(
        (server): server is string => typeof server === "string" && SERVER_NAME.test(server)
      )
    : []
  return {
    id,
    name: name.trim(),
    description: description.trim(),
    tags: (tags as string[]).map((tag) => tag.trim()),
    kind: kind as TemplateKind,
    ...(requires ? { requires } : {}),
    ...(platforms ? { architectures: platforms } : {}),
    ...(servers.length ? { mcp: servers } : {}),
  }
}

/** Reads the list: the entries the page can offer, and the reasons for those it cannot. */
export function readTemplateList(value: unknown): TemplateList {
  const templates = (value as { templates?: unknown } | null)?.templates
  if (!Array.isArray(templates)) throw new Error(t("templates.listInvalid"))
  const list: TemplateList = { templates: [], broken: [] }
  const seen = new Set<string>()
  templates.forEach((entry, index) => {
    const id = String((entry as { id?: unknown } | null)?.id ?? `#${index + 1}`)
    try {
      const template = readTemplate(entry)
      if (seen.has(template.id)) throw new Error(t("templates.broken.duplicate"))
      seen.add(template.id)
      list.templates.push(template)
    } catch (error) {
      list.broken.push({ id, reason: (error as Error).message })
    }
  })
  return list
}

/** Compares two `major.minor.patch` versions; anything after the patch number is ignored. */
export function compareVersions(a: string, b: string) {
  const parts = (version: string) =>
    version
      .split(/[^\d.]/)[0]
      .split(".")
      .map((part) => Number(part) || 0)
  const [left, right] = [parts(a), parts(b)]
  for (let index = 0; index < 3; index++) {
    const difference = (left[index] ?? 0) - (right[index] ?? 0)
    if (difference) return Math.sign(difference)
  }
  return 0
}

/** Whether this manager can build the template. */
export const buildable = (template: Pick<Template, "requires">, version: string) =>
  !template.requires || compareVersions(version, template.requires) >= 0

/** The architectures a template builds on. */
export const architecturesOf = (template: Pick<Template, "architectures">) =>
  template.architectures ?? DEFAULT_ARCHITECTURES

/**
 * Whether a template builds on a Docker host that runs `architecture` images, the
 * `architecture` of `/api/settings`. One that downloads programs made for another architecture
 * fails its build there, or builds an image whose programs do not start.
 */
export const fits = (template: Pick<Template, "architectures">, architecture: string) =>
  architecturesOf(template).includes(architecture)

/** A template's file beside the list: its configuration or its icon. */
export const templateFile = (list: string, id: string, extension: "json" | "png") =>
  new URL(`${encodeURIComponent(id)}.${extension}`, list).href

/** Whether a template matches a search, by its name, description or tags. */
export function matches(template: Template, search: string) {
  const words = search.toLocaleLowerCase().split(/\s+/).filter(Boolean)
  const text = [template.name, template.description, ...template.tags].join(" ").toLocaleLowerCase()
  return words.every((word) => text.includes(word))
}

/**
 * The tags worth filtering by, each with how many templates carry it, the most common first. A tag
 * of one template finds nothing the search does not, and one every template carries narrows
 * nothing, so neither is offered; both still show on their cards and match the search.
 */
export function sharedTags(templates: Template[]) {
  const counts = new Map<string, number>()
  for (const template of templates)
    for (const tag of new Set(template.tags)) counts.set(tag, (counts.get(tag) ?? 0) + 1)
  return [...counts]
    .filter(([, count]) => count > 1 && count < templates.length)
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name))
}

/**
 * The templates of each kind that has any, applications first, each in the list's order. The page
 * shows one section per group.
 */
export const groupByKind = (templates: Template[]) =>
  TEMPLATE_KINDS.map((kind) => ({
    kind,
    templates: templates.filter((template) => template.kind === kind),
  })).filter((group) => group.templates.length > 0)

// The list is public, so requests carry no cookies, and they ask the server whether a cached copy
// is still current, since a change on the wiki should show without waiting for the cache to expire.
async function fetchJson(url: string): Promise<unknown> {
  let response: Response
  try {
    response = await fetch(url, { credentials: "omit", cache: "no-cache" })
  } catch {
    throw new Error(t("templates.unreachable"))
  }
  if (!response.ok) throw new Error(t("templates.fetchFailed", { status: response.status }))
  try {
    return await response.json()
  } catch {
    throw new Error(t("templates.notJson"))
  }
}

export async function fetchTemplateList(list: string) {
  return readTemplateList(await fetchJson(list))
}

/**
 * A template as the build form takes it. The icon is the list's `<id>.png`, the picture its card
 * showed; one inside the configuration is used only when that file cannot be read.
 */
export async function loadTemplate(list: string, template: Template): Promise<ImportedBuild> {
  let config: ImportedBuild
  try {
    config = readBuildConfig(await fetchJson(templateFile(list, template.id, "json")))
  } catch (error) {
    throw new Error(
      t("templates.invalid", { name: template.name, reason: (error as Error).message }),
      { cause: error }
    )
  }
  const icon = await fetch(templateFile(list, template.id, "png"), { credentials: "omit" })
    .then((response) => (response.ok ? response.blob() : undefined))
    .then((blob) => blob && iconFromBlob(blob))
    .catch(() => undefined)
  return {
    ...config,
    name: config.name || template.name,
    description: config.description || template.description,
    ...(icon || config.icon ? { icon: icon ?? config.icon } : {}),
    template_id: template.id,
  }
}

/** A template with the configuration its Install fills in. */
export type LoadedTemplate = { template: Template; config: ImportedBuild }

const clip = (text: string, characters: number) => [...text].slice(0, characters).join("")

/** `text` cut to at most `limit` bytes of UTF-8, at a character's end. */
function clipBytes(text: string, limit: number) {
  const encoder = new TextEncoder()
  const characters = [...clip(text, limit)]
  while (encoder.encode(characters.join("")).length > limit) characters.pop()
  return characters.join("")
}

/**
 * One build of several templates, in the order given. Each template's script runs as a part, in
 * a layer of its own, and each compose becomes a step of the image's compose, whose script
 * receives only its own parameters, so two templates may ask for the same name. The image is
 * built on the first template's base; a template made for another one is a warning, since its
 * script may expect that desktop. A template merged from others brings its parts and steps.
 * The desktop MCP servers they declare are all written into the image, so two templates may not
 * declare one name. A template's options stay with its part, whose script gets their values.
 * A template that does not build on the Docker host's `architecture` is refused, since its part
 * would fail the whole build.
 */
export function mergeTemplates(loaded: LoadedTemplate[], architecture: string): ImportedBuild {
  const parts: DraftPart[] = []
  const steps: ComposeStep[] = []
  const servers: ServerDeclaration[] = []
  // Which template brought each server, to name both when another declares it again.
  const owners = new Map<string, string>()
  const warnings: string[] = []
  const base = loaded.find(({ config }) => config.base_image)?.config.base_image
  const add = <T extends { id?: string; name?: string }>(list: T[], item: T) => {
    if (list.some((other) => other.id === item.id))
      throw new Error(t("templates.merge.twice", { name: item.name || item.id || "" }))
    list.push(item)
  }
  for (const { template, config } of loaded) {
    if (!fits(template, architecture))
      throw new Error(
        t("templates.merge.architecture", {
          name: template.name,
          architectures: formatList(architecturesOf(template)),
          architecture,
        })
      )
    for (const part of config.parts ?? [
      {
        id: template.id,
        name: template.name,
        script: config.script ?? "",
        ...(config.options?.length ? { options: config.options } : {}),
      },
    ])
      add(parts, part)
    const own = config.compose ? composeSteps(config.compose) : []
    for (const step of own)
      add(
        steps,
        own.length > 1 ? step : { ...step, id: template.id, name: step.name || template.name }
      )
    for (const server of config.mcp ?? []) {
      const owner = owners.get(server.name)
      if (owner !== undefined)
        throw new Error(
          t("templates.merge.mcpTwice", { name: server.name, first: owner, second: template.name })
        )
      owners.set(server.name, template.name)
      servers.push(server)
    }
    if (config.base_image && base && config.base_image !== base)
      warnings.push(t("templates.merge.base", { name: template.name, base: config.base_image }))
  }
  const icon = loaded.find(({ config }) => config.icon)?.config.icon
  const description = loaded
    .map(({ template, config }) => config.description || template.description)
    .filter(Boolean)
    .join("\n\n")
  return {
    name: clip(loaded.map(({ template }) => template.name).join(" + "), 100),
    description: clipBytes(description, 4096),
    ...(base ? { base_image: base } : {}),
    internet: loaded.some(({ config }) => config.internet !== false),
    parts,
    ...(steps.length ? { compose: { steps } } : {}),
    ...(servers.length ? { mcp: servers } : {}),
    ...(icon ? { icon } : {}),
    template_ids: [
      ...new Set(
        loaded.flatMap(({ template, config }) => [template.id, ...(config.template_ids ?? [])])
      ),
    ],
    ...(warnings.length ? { warnings } : {}),
  }
}
