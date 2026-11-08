// What the MCP inspector shows of a server: its tools, prompts, resources and resource
// templates, the arguments each takes, and how a result reads. Pure helpers; the connection is
// in mcp-client.ts.
import type { Concept } from "./concepts"
import { stateLabel } from "./format"
import { formatNumber, t } from "./i18n"
import type { InspectTarget } from "./mcp-client"
import { isPlainObject, prettyJson, type FieldChoices, type JsonSchema } from "./schema-form"
import type { ContainerItem, DesktopServer, McpEndpoint } from "./types"

/** A server the inspector can open, as the picker offers it. */
export type TargetChoice = {
  key: string
  target: InspectTarget
  label: string
  /** Why it cannot be opened now. */
  unavailable?: string
}
/** A group of the picker; `concept` gives its heading an icon, a container's above its servers. */
export type TargetGroup = { label: string; concept?: Concept; choices: TargetChoice[] }

/** The picker's key of a target. */
export function targetKey(target: InspectTarget): string {
  switch (target.kind) {
    case "administrator":
      return "administrator"
    case "endpoint":
      return `endpoint:${target.name}`
    case "server":
      return `server:${target.container}:${target.name}`
    case "trial":
      return `trial:${target.container}:${target.server.name}`
  }
}

/**
 * What the inspector can open: Thoric's own MCP while the root /mcp is on, the endpoints, which
 * answer either way, and the desktop servers of the running containers. A stopped container's
 * servers are listed too, so it is clear why they cannot be opened.
 */
export function targetGroups(
  servers: DesktopServer[],
  endpoints: McpEndpoint[],
  containers: Pick<ContainerItem, "id" | "state" | "spec">[],
  rootMcpEnabled: boolean
): TargetGroup[] {
  const off = rootMcpEnabled ? undefined : t("inspector.target.rootOff")
  const running = new Set(containers.filter((c) => c.state === "running").map((c) => c.id))
  const names = new Map(containers.map((c) => [c.id, c.spec.name]))
  const groups: TargetGroup[] = [
    {
      label: t("inspector.target.thoric"),
      choices: [
        {
          key: "administrator",
          target: { kind: "administrator" },
          label: t("inspector.target.administrator"),
          unavailable: off,
        },
      ],
    },
  ]
  if (endpoints.length)
    groups.push({
      label: t("nav.mcp-endpoints"),
      concept: "mcp",
      choices: [...endpoints]
        .sort((a, b) => a.name.localeCompare(b.name))
        .map((endpoint) => {
          const target: InspectTarget = { kind: "endpoint", name: endpoint.name }
          return { key: targetKey(target), target, label: endpoint.path }
        }),
    })
  const byContainer = new Map<string, DesktopServer[]>()
  for (const server of servers)
    byContainer.set(server.container_id, [...(byContainer.get(server.container_id) ?? []), server])
  for (const [container, list] of byContainer)
    groups.push({
      label: names.get(container) ?? list[0].container_name,
      concept: "container",
      choices: [...list]
        .sort((a, b) => a.name.localeCompare(b.name))
        .map((server) => {
          const target: InspectTarget = { kind: "server", container, name: server.name }
          return {
            key: targetKey(target),
            target,
            label: server.name,
            unavailable: !running.has(container)
              ? t("inspector.target.stopped")
              : server.error && !server.command && !server.url
                ? server.error
                : undefined,
          }
        }),
    })
  return groups
}

/**
 * What the text fields of the opened server offer beside typing, by the field's name. Thoric's
 * own MCP takes a container by its Thoric ID in `container_id`, so it offers every container by
 * name. An endpoint names its containers in an enum of its own, and a desktop server's fields are
 * its own, whatever they are called.
 */
export function fieldChoices(
  target: InspectTarget,
  containers: Pick<ContainerItem, "id" | "state" | "spec">[]
): ReadonlyMap<string, FieldChoices> {
  if (target.kind !== "administrator") return new Map()
  return new Map([
    [
      "container_id",
      {
        label: t("inspector.form.pickContainer"),
        empty: t("inspector.form.noContainers"),
        concept: "container",
        items: [...containers]
          .sort((a, b) => a.spec.name.localeCompare(b.spec.name))
          .map((container) => ({
            value: container.id,
            label: container.spec.name,
            detail: stateLabel(container.state),
          })),
      },
    ],
  ])
}

export type ItemKind = "tool" | "prompt" | "resource" | "template"
export const ITEM_KINDS: ItemKind[] = ["tool", "prompt", "resource", "template"]

/** A tool as `tools/list` gives it. */
export type ToolItem = {
  name: string
  title?: string
  description?: string
  inputSchema?: JsonSchema
  outputSchema?: JsonSchema
  annotations?: {
    title?: string
    readOnlyHint?: boolean
    destructiveHint?: boolean
    idempotentHint?: boolean
    openWorldHint?: boolean
  }
}
export type PromptItem = {
  name: string
  title?: string
  description?: string
  arguments?: { name: string; title?: string; description?: string; required?: boolean }[]
}
export type ResourceItem = {
  uri: string
  name?: string
  title?: string
  description?: string
  mimeType?: string
  size?: number
}
export type TemplateItem = {
  uriTemplate: string
  name?: string
  title?: string
  description?: string
  mimeType?: string
}
export type Item = ToolItem | PromptItem | ResourceItem | TemplateItem
export type Lists = {
  tool: ToolItem[]
  prompt: PromptItem[]
  resource: ResourceItem[]
  template: TemplateItem[]
}

/** What tells an item apart from the others of its kind: a name, a URI or a URI template. */
export function itemKey(kind: ItemKind, item: Item): string {
  if (kind === "resource") return (item as ResourceItem).uri
  if (kind === "template") return (item as TemplateItem).uriTemplate
  return (item as ToolItem).name
}

/** An item's id across the kinds. */
export const itemId = (kind: ItemKind, key: string) => `${kind}:${key}`

/** The name an item is listed by. */
export function itemName(kind: ItemKind, item: Item): string {
  if (kind === "resource" || kind === "template")
    return (item as ResourceItem).name || itemKey(kind, item)
  return (item as ToolItem).name
}

/** A title the server gives besides the name, when it differs. */
export function itemTitle(kind: ItemKind, item: Item): string | undefined {
  const title = item.title || (kind === "tool" ? (item as ToolItem).annotations?.title : undefined)
  return title && title !== itemName(kind, item) ? title : undefined
}

export type Sort = "server" | "ascending" | "descending"

/** The items a search keeps, by name, title or URI, in the order chosen. */
export function filterItems<T extends Item>(
  kind: ItemKind,
  items: T[],
  query: string,
  sort: Sort
): T[] {
  const words = query.trim().toLowerCase()
  const kept = words
    ? items.filter((item) =>
        [itemName(kind, item), item.title, itemKey(kind, item)].some((text) =>
          text?.toLowerCase().includes(words)
        )
      )
    : [...items]
  if (sort !== "server") {
    kept.sort((a, b) => itemName(kind, a).localeCompare(itemName(kind, b)))
    if (sort === "descending") kept.reverse()
  }
  return kept
}

/** The names a URI template fills in, such as `owner` and `repo` in `repos/{owner}/{repo}`. */
export function templateVariables(template: string): string[] {
  const names = new Set<string>()
  for (const match of template.matchAll(/\{[+#./;?&]?([^}]+)\}/g))
    for (const part of match[1].split(","))
      names.add(part.replace(/\*$/, "").replace(/:\d+$/, "").trim())
  return [...names].filter(Boolean)
}

const OPERATORS: Record<
  string,
  { first: string; separator: string; named: boolean; empty: string; reserved: boolean }
> = {
  "": { first: "", separator: ",", named: false, empty: "", reserved: false },
  "+": { first: "", separator: ",", named: false, empty: "", reserved: true },
  "#": { first: "#", separator: ",", named: false, empty: "", reserved: true },
  ".": { first: ".", separator: ".", named: false, empty: "", reserved: false },
  "/": { first: "/", separator: "/", named: false, empty: "", reserved: false },
  ";": { first: ";", separator: ";", named: true, empty: "", reserved: false },
  "?": { first: "?", separator: "&", named: true, empty: "=", reserved: false },
  "&": { first: "&", separator: "&", named: true, empty: "=", reserved: false },
}

const encode = (value: string, reserved: boolean) =>
  reserved
    ? encodeURI(value).replace(/%25([0-9A-Fa-f]{2})/g, "%$1")
    : encodeURIComponent(value).replace(
        /[!'()*]/g,
        (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`
      )

/**
 * A URI template (RFC 6570) filled in with `values`; a variable left out disappears with its
 * operator's punctuation. A value may be a list, from a JSON array, for a `*` variable.
 */
export function expandTemplate(template: string, values: Record<string, string>): string {
  return template.replace(/\{([+#./;?&]?)([^}]+)\}/g, (_, operator: string, list: string) => {
    const rule = OPERATORS[operator]
    const parts: string[] = []
    for (const spec of list.split(",")) {
      const explode = spec.trim().endsWith("*")
      const [name, prefix] = spec.trim().replace(/\*$/, "").split(":")
      const raw = values[name]
      if (raw === undefined || raw === "") {
        if (raw === "" && rule.named) parts.push(`${name}${rule.empty}`)
        continue
      }
      let items: string[] | undefined
      if (explode || raw.startsWith("[")) {
        try {
          const parsed: unknown = JSON.parse(raw)
          if (Array.isArray(parsed)) items = parsed.map(String)
        } catch {
          // A value that is not a JSON list is one value.
        }
      }
      if (items) {
        const encoded = items.map((item) => encode(item, rule.reserved))
        if (explode) parts.push(...encoded.map((item) => (rule.named ? `${name}=${item}` : item)))
        else parts.push(rule.named ? `${name}=${encoded.join(",")}` : encoded.join(","))
        continue
      }
      const text = prefix ? [...raw].slice(0, Number(prefix)).join("") : raw
      const encoded = encode(text, rule.reserved)
      parts.push(rule.named ? `${name}=${encoded}` : encoded)
    }
    return parts.length ? rule.first + parts.join(rule.separator) : ""
  })
}

/** The schema of what an item takes: a tool's own, a prompt's arguments or a template's names. */
export function inputSchemaOf(kind: ItemKind, item: Item): JsonSchema {
  switch (kind) {
    case "tool":
      return (item as ToolItem).inputSchema ?? { type: "object" }
    case "prompt": {
      const args = (item as PromptItem).arguments ?? []
      return {
        type: "object",
        properties: Object.fromEntries(
          args.map((arg) => [
            arg.name,
            { type: "string", title: arg.title, description: arg.description },
          ])
        ),
        required: args.filter((arg) => arg.required).map((arg) => arg.name),
        additionalProperties: false,
      }
    }
    case "template": {
      const names = templateVariables((item as TemplateItem).uriTemplate)
      return {
        type: "object",
        properties: Object.fromEntries(names.map((name) => [name, { type: "string" }])),
        required: names,
        additionalProperties: false,
      }
    }
    case "resource":
      return { type: "object", properties: {}, additionalProperties: false }
  }
}

/** Arguments as prompts and templates take them: every value a string. */
export function stringValues(args: Record<string, unknown>): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [name, value] of Object.entries(args))
    if (value !== undefined && value !== null)
      out[name] = typeof value === "string" ? value : JSON.stringify(value)
  return out
}

/** JSON with sorted keys, so two values compare whatever the order of their keys. */
export function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`
  if (isPlainObject(value))
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(",")}}`
  return JSON.stringify(value) ?? "null"
}

/** Whether `text` is the JSON of `value`, as a text block repeating structuredContent is. */
export function sameJson(text: string, value: unknown) {
  try {
    return canonical(JSON.parse(text)) === canonical(value)
  } catch {
    return false
  }
}

/** `text` laid out as JSON when it is an object or an array, otherwise nothing. */
export function asJson(text: string): string | null {
  const trimmed = text.trim()
  if (!/^[[{]/.test(trimmed)) return null
  try {
    return prettyJson(JSON.parse(trimmed))
  } catch {
    return null
  }
}

/** The longest string shown whole in a raw result; base64 data is usually far longer. */
const SHOWN = 1000

/** `value` with its very long strings, such as base64 images, shortened for showing. */
export function shortenStrings(value: unknown): unknown {
  if (typeof value === "string")
    return value.length > SHOWN
      ? `${value.slice(0, 120)}… ${t("inspector.result.characters", { count: formatNumber(value.length) })}`
      : value
  if (Array.isArray(value)) return value.map(shortenStrings)
  if (isPlainObject(value))
    return Object.fromEntries(
      Object.entries(value).map(([key, inner]) => [key, shortenStrings(inner)])
    )
  return value
}

/** A data URL for base64 content, or nothing when the type or the data are not plain. */
export function mediaSource(mimeType: unknown, data: unknown): string | null {
  if (typeof mimeType !== "string" || !/^[\w.+-]+\/[\w.+-]+$/.test(mimeType)) return null
  if (typeof data !== "string" || !/^[A-Za-z0-9+/=\s]*$/.test(data)) return null
  return `data:${mimeType};base64,${data.replace(/\s/g, "")}`
}

/** How many bytes base64 data holds. */
export const base64Bytes = (data: string) => {
  const clean = data.replace(/\s/g, "")
  return (
    Math.floor((clean.length * 3) / 4) - (clean.endsWith("==") ? 2 : clean.endsWith("=") ? 1 : 0)
  )
}

/** How long a request took. */
export const requestDuration = (ms: number) =>
  ms < 1000
    ? t("inspector.result.milliseconds", { value: formatNumber(Math.round(ms)) })
    : t("inspector.result.seconds", {
        value: formatNumber(ms / 1000, { minimumFractionDigits: 2, maximumFractionDigits: 2 }),
      })

/** The file name a result is saved under. */
export const resultFile = (kind: ItemKind, key: string) =>
  `${kind}-${key.replace(/[^A-Za-z0-9._-]+/g, "_").slice(0, 80) || "result"}.json`

/** The annotations a tool claims, as short labels; destructive only when not read-only. */
export function toolHints(tool: ToolItem): string[] {
  const hints = tool.annotations ?? {}
  return [
    hints.readOnlyHint === true && "readOnly",
    hints.destructiveHint === true && hints.readOnlyHint !== true && "destructive",
    hints.idempotentHint === true && "idempotent",
    hints.openWorldHint === true && "openWorld",
    tool.outputSchema && "structured",
  ].filter((hint): hint is string => !!hint)
}
