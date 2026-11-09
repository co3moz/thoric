// MCP endpoints and desktop MCP servers as the MCP pages and the forms show them. Thoric MCP at
// `/mcp` is the administrator channel; an endpoint at `/mcp/<name>` gives an agent some of the
// container tools of the containers it covers, plus the tools of their desktop MCP servers.
import type { Tone } from "./format"
import { formatList, t } from "./i18n"
import type {
  DesktopServer,
  EndpointTool,
  GrantRequest,
  ImageItem,
  McpGrant,
  McpProfile,
  McpScope,
  McpScopes,
  ServerDeclaration,
  ServerRequest,
} from "./types"

/** Every scope, in the order the forms list them. */
export const SCOPES: McpScope[] = [
  "view",
  "input",
  "applications",
  "files_read",
  "files_write",
  "commands",
  "media",
  "power",
  "scale",
]
/** The scopes a container's grant may have: every one but `scale`, which only a scale set's has. */
export const CONTAINER_SCOPES = SCOPES.filter((scope) => scope !== "scale")

/** Every tool profile, in the order the forms list them; the first is what a grant has by default. */
export const PROFILES: McpProfile[] = ["full", "headless", "desktop"]

/** A profile's name; one a newer manager added as it calls it. */
export const profileLabel = (profile: string) =>
  PROFILES.includes(profile as McpProfile) ? t(`mcp.profile.${profile as McpProfile}`) : profile
/** What a profile keeps; nothing for one this release does not know. */
export const profileDescription = (profile: string) =>
  PROFILES.includes(profile as McpProfile)
    ? t(`mcp.profile.${profile as McpProfile}.description`)
    : ""

/**
 * Whether a profile keeps the Thoric tool `name`, by the manager's lists. `full` keeps every
 * tool, and so does any profile while the manager's lists are not known.
 */
export function inProfile(profile: McpProfile, name: string, scopes?: McpScopes) {
  if (profile === "full") return true
  const tools = scopes?.profiles?.find((entry) => entry.id === profile)?.tools
  return !tools || tools.includes(name)
}

/**
 * The Thoric tools a grant offers: those of its scopes that `profile` keeps, `get_job` with a
 * scope whose tools return a job, and for a scale set the tools its endpoint always has, each
 * unless the grant leaves it out by name.
 */
export function thoricTools(
  draft: Pick<GrantDraft, "scopes" | "excluded" | "set">,
  scopes: McpScopes | undefined,
  profile: McpProfile
): string[] {
  if (!scopes) return []
  const granted = scopes.scopes.filter((scope) => draft.scopes.includes(scope.id))
  const names = [
    ...(draft.set ? (scopes.set_tools ?? []) : []),
    ...granted.flatMap((scope) => scope.tools),
    ...(granted.some((scope) => scopes.job_scopes?.includes(scope.id)) ? ["get_job"] : []),
  ]
  return [...new Set(names)].filter(
    (name) => inProfile(profile, name, scopes) && !draft.excluded.includes(name)
  )
}

/**
 * How many tools of the desktop servers a grant offers, as the manager last read them: those of
 * the servers it includes, without the ones it leaves out by name.
 */
export function serverToolCount(
  draft: Pick<GrantDraft, "all" | "servers" | "excluded">,
  servers: Pick<DesktopServer, "name" | "tools">[]
) {
  return servers
    .filter((server) => draft.all || draft.servers.includes(server.name))
    .reduce(
      (count, server) =>
        count + server.tools.filter((tool) => !draft.excluded.includes(tool.exposed)).length,
      0
    )
}

/** A desktop MCP server's name: lowercase letters, digits and dashes, at most 32 characters. */
export const SERVER_NAME = /^[a-z0-9][a-z0-9-]{0,31}$/
/** The same rule for an input's `pattern`, which the browser matches with the `v` flag. */
export const SERVER_NAME_PATTERN = "[a-z0-9][a-z0-9\\-]{0,31}"
/** An endpoint's name, the last part of its URL: the same letters, at most 48 characters. */
export const ENDPOINT_NAME = /^[a-z0-9][a-z0-9-]{0,47}$/
export const ENDPOINT_NAME_PATTERN = "[a-z0-9][a-z0-9\\-]{0,47}"
/** The name an endpoint gives a container: the same letters, at most 24 characters. */
export const ALIAS_PATTERN = "[a-z0-9][a-z0-9\\-]{0,23}"
const ALIAS_LIMIT = 24

const known = (scope: string): scope is McpScope => SCOPES.includes(scope as McpScope)

/** A scope's name; one a newer manager added as it calls it. */
export const scopeLabel = (scope: string) => (known(scope) ? t(`mcp.scope.${scope}`) : scope)
/** What a scope lets an agent do; nothing for one this release does not know. */
export const scopeDescription = (scope: string) =>
  known(scope) ? t(`mcp.scope.${scope}.description`) : ""

/** Scopes in the forms' order, each once; unknown ones a newer manager sends are kept last. */
export function orderScopes(scopes: readonly string[]): McpScope[] {
  const other = scopes.filter((scope) => !known(scope)) as McpScope[]
  return [...SCOPES.filter((scope) => scopes.includes(scope)), ...new Set(other)]
}

/** A grant's scopes in a few words. */
export function scopeSummary(scopes: readonly McpScope[]) {
  if (!scopes.length) return t("mcp.scopes.none")
  if (CONTAINER_SCOPES.every((scope) => scopes.includes(scope))) return t("mcp.scopes.all")
  return formatList(orderScopes(scopes).map(scopeLabel))
}

/** Which desktop MCP servers a grant includes, in a few words. */
export function serversSummary(servers: McpGrant["servers"]) {
  if (servers === "all") return t("mcp.servers.all")
  return servers.length ? formatList(servers) : t("mcp.servers.none")
}

/**
 * The name an endpoint gives a container by default: its name in lowercase letters, digits and
 * dashes, with a number added when another container of the endpoint already goes by it.
 */
export function defaultAlias(name: string, taken: Iterable<string> = []) {
  const base =
    name
      .replace(/ı/g, "i")
      .normalize("NFKD")
      .replace(/[\u0300-\u036f]/g, "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+/, "")
      .slice(0, ALIAS_LIMIT)
      .replace(/-+$/, "") || "desktop"
  const used = new Set(taken)
  if (!used.has(base)) return base
  for (let number = 2; ; number++) {
    const suffix = `-${number}`
    const candidate = `${base.slice(0, ALIAS_LIMIT - suffix.length).replace(/-+$/, "")}${suffix}`
    if (!used.has(candidate)) return candidate
  }
}

/**
 * One container of the endpoint form. `all` keeps the servers chosen before it was turned on,
 * and `excluded` keeps a tool left out while its scope or server is off, so turning it on again
 * brings the choice back.
 */
export type GrantDraft = {
  container_id: string
  /** The scale set the grant covers instead of a container. */
  set?: string
  container_name: string
  alias: string
  scopes: McpScope[]
  /** Which of the scopes' tools the grant offers. */
  profile: McpProfile
  all: boolean
  servers: string[]
  /** Tools left out, by the name the endpoint gives them for this container alone. */
  excluded: string[]
}

export const grantDraft = (grant: McpGrant): GrantDraft => ({
  container_id: grant.container_id,
  ...(grant.set ? { set: grant.set } : {}),
  container_name: grant.container_name,
  alias: grant.alias,
  scopes: orderScopes(grant.scopes),
  profile: grant.profile ?? "full",
  all: grant.servers === "all",
  servers: grant.servers === "all" ? [] : [...grant.servers],
  excluded: [...grant.excluded_tools],
})

/** Whether an exposed tool name is a desktop server's, `SERVER__TOOL`; a container tool's has no `__`. */
export const isServerTool = (name: string) => name.includes("__")

/**
 * The tools a grant leaves out, as a request sends them, each once. A container tool's name goes
 * only while its scope is granted, since the manager refuses one of a scope the grant does not
 * have; without the manager's scopes it cannot be told, and every name goes. A desktop server's
 * tool goes whatever the servers chosen: it is kept for a server that comes back.
 */
export function excludedTools(
  draft: Pick<GrantDraft, "scopes" | "excluded">,
  scopes?: McpScopes
): string[] {
  const granted = new Set(
    scopes?.scopes
      .filter((scope) => draft.scopes.includes(scope.id))
      .flatMap((scope) => scope.tools) ?? []
  )
  return [...new Set(draft.excluded)].filter(
    (name) => isServerTool(name) || !scopes || granted.has(name)
  )
}

/** A container added to the form: the default scopes and every desktop MCP server. */
export const newGrant = (
  container: { id: string; name: string },
  scopes: readonly McpScope[],
  taken: Iterable<string>
): GrantDraft => ({
  container_id: container.id,
  container_name: container.name,
  alias: defaultAlias(container.name, taken),
  scopes: orderScopes(scopes),
  profile: "full",
  all: true,
  servers: [],
  excluded: [],
})

/**
 * The grants an endpoint request sends, or the reason the form cannot send them. `scopes`, the
 * manager's, tells which excluded container tools still have their scope.
 */
export function grantRequests(drafts: GrantDraft[], scopes?: McpScopes): GrantRequest[] {
  if (!drafts.length) throw new Error(t("mcp.endpoint.needGrant"))
  const seen = new Set<string>()
  return drafts.map((draft) => {
    const alias = draft.alias.trim()
    if (alias) {
      if (seen.has(alias)) throw new Error(t("mcp.endpoint.aliasTwice", { alias }))
      seen.add(alias)
    }
    const excluded = excludedTools(draft, scopes)
    return {
      container_id: draft.container_id,
      ...(draft.set ? { set: draft.set } : {}),
      ...(alias ? { alias } : {}),
      scopes: orderScopes(draft.scopes),
      // `full` sends none, as requests did before there were profiles.
      ...(draft.profile !== "full" ? { profile: draft.profile } : {}),
      servers: draft.all ? "all" : draft.servers,
      ...(excluded.length ? { excluded_tools: excluded } : {}),
    }
  })
}

/** Where an endpoint answers, from the dashboard's origin and the path the manager reports. */
export const endpointUrl = (origin: string, path: string) =>
  `${origin.replace(/\/+$/, "")}${path.startsWith("/") ? path : `/${path}`}`

/** The endpoint routes of the manager's API. */
export const endpointApi = (name?: string) =>
  name ? `mcp/endpoints/${encodeURIComponent(name)}` : "mcp/endpoints"

/** A container's desktop MCP servers in the manager's API, or one of them. */
export const serverApi = (container: string, name?: string) =>
  `containers/${encodeURIComponent(container)}/mcp/servers${name ? `/${encodeURIComponent(name)}` : ""}`

/** The color family of a desktop MCP server's state. */
export function serverTone(state: DesktopServer["state"]): Tone {
  if (state === "ready") return "success"
  if (state === "error") return "danger"
  if (state === "pending") return "warning"
  // `stopped`, and `elsewhere`, which is how the set wants it.
  return "muted"
}

/** How a server starts, as one line: its command with its arguments, or its URL. */
export function serverCommand(server: Pick<ServerRequest, "command" | "args" | "url">) {
  if (!server.command) return server.url ?? ""
  const quote = (part: string) => (!part || /[\s"'\\]/.test(part) ? JSON.stringify(part) : part)
  return [server.command, ...(server.args ?? [])].map(quote).join(" ")
}

/** What else a server is given, by name only: its environment, its headers and its folder. */
export function serverDetails(env: string[], headers: string[], cwd?: string | null) {
  return [
    env.length ? t("mcp.server.envNames", { names: formatList(env) }) : "",
    headers.length ? t("mcp.server.headerNames", { names: formatList(headers) }) : "",
    cwd ? t("mcp.server.cwdIn", { path: cwd }) : "",
  ]
    .filter(Boolean)
    .join(" · ")
}

/** The servers an image declares, by an image ID or reference as the create form names it. */
export function imageServers(
  images: Pick<ImageItem, "image_id" | "reference" | "mcp_servers">[],
  value?: string
) {
  if (!value) return []
  return (
    images.find(
      (image) =>
        image.mcp_servers?.length && (image.image_id === value || image.reference === value)
    )?.mcp_servers ?? []
  )
}

/** The desktop MCP servers by container, in the order of `containers`; others follow. */
export function serversByContainer<C extends { id: string }>(
  servers: DesktopServer[],
  containers: C[]
) {
  const groups = new Map<string, DesktopServer[]>()
  for (const server of servers) {
    const list = groups.get(server.container_id) ?? []
    list.push(server)
    groups.set(server.container_id, list)
  }
  const known = new Set(containers.map((container) => container.id))
  return [
    ...containers.map((container) => ({
      id: container.id,
      container: container as C | undefined,
      name: groups.get(container.id)?.[0]?.container_name ?? "",
      servers: groups.get(container.id) ?? [],
    })),
    ...[...groups]
      .filter(([id]) => !known.has(id))
      .map(([id, list]) => ({
        id,
        container: undefined as C | undefined,
        name: list[0].container_name,
        servers: list,
      })),
  ]
}

/**
 * An endpoint's tools by where they come from: Thoric's or one server's, per container.
 * A tool of the endpoint itself, such as `get_job`, has no container and forms a group of its own.
 */
export function toolGroups(tools: EndpointTool[]) {
  const groups = new Map<
    string,
    {
      source: EndpointTool["source"]
      container_id: string | null
      server: string | null
      tools: EndpointTool[]
    }
  >()
  for (const tool of tools) {
    const key = `${tool.source}\u0000${tool.container_id ?? ""}\u0000${tool.server ?? ""}`
    const group = groups.get(key) ?? {
      source: tool.source,
      container_id: tool.container_id,
      server: tool.server,
      tools: [],
    }
    group.tools.push(tool)
    groups.set(key, group)
  }
  return [...groups.values()]
}

/** One argument per line; blank lines are left out. */
export const parseLines = (text: string) =>
  text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)

const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/
const HEADER_NAME = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/

/** `KEY=VALUE` lines, the value everything after the first `=`, or why one cannot be read. */
export function parseEnv(text: string): Record<string, string> {
  const values: Record<string, string> = {}
  text.split(/\r?\n/).forEach((line, index) => {
    if (!line.trim()) return
    const at = line.indexOf("=")
    const name = at < 0 ? "" : line.slice(0, at).trim()
    if (!ENV_NAME.test(name)) throw new Error(t("mcp.server.envLine", { line: index + 1 }))
    if (Object.hasOwn(values, name)) throw new Error(t("mcp.server.twice", { name }))
    values[name] = line.slice(at + 1)
  })
  return values
}

/** `Name: value` lines, or why one cannot be read. Header names do not differ by case. */
export function parseHeaders(text: string): Record<string, string> {
  const values: Record<string, string> = {}
  const seen = new Set<string>()
  text.split(/\r?\n/).forEach((line, index) => {
    if (!line.trim()) return
    const at = line.indexOf(":")
    const name = at < 0 ? "" : line.slice(0, at).trim()
    if (!HEADER_NAME.test(name)) throw new Error(t("mcp.server.headerLine", { line: index + 1 }))
    if (seen.has(name.toLowerCase())) throw new Error(t("mcp.server.twice", { name }))
    seen.add(name.toLowerCase())
    values[name] = line.slice(at + 1).trim()
  })
  return values
}

/**
 * Whether a URL points at a server inside the desktop: http or https on 127.0.0.1, localhost or
 * [::1], with a port. A compose value such as `${PORT}` may stand for any part after the host.
 */
export const localUrl = (url: string) =>
  /^https?:\/\/(127\.0\.0\.1|localhost|\[::1\]):[^/?#\s]+([/?#]\S*)?$/i.test(url.trim())

/** The Add server form, as text fields. */
export type ServerDraft = {
  name: string
  description: string
  transport: "stdio" | "http"
  command: string
  args: string
  env: string
  cwd: string
  url: string
  headers: string
}

export const serverDraft = (): ServerDraft => ({
  name: "",
  description: "",
  transport: "stdio",
  command: "",
  args: "",
  env: "",
  cwd: "",
  url: "",
  headers: "",
})

/** The request the Add server form sends, or the reason it cannot send one. */
export function serverRequest(draft: ServerDraft): ServerRequest {
  const name = draft.name.trim()
  if (!SERVER_NAME.test(name)) throw new Error(t("mcp.server.nameInvalid"))
  const description = draft.description.trim()
  const head = { name, ...(description ? { description } : {}) }
  if (draft.transport === "http") {
    const url = draft.url.trim()
    if (!localUrl(url)) throw new Error(t("mcp.server.urlLocal"))
    const headers = parseHeaders(draft.headers)
    return { ...head, url, ...(Object.keys(headers).length ? { headers } : {}) }
  }
  const command = draft.command.trim()
  if (!command) throw new Error(t("mcp.server.commandMissing"))
  const args = parseLines(draft.args)
  const env = parseEnv(draft.env)
  const cwd = draft.cwd.trim()
  return {
    ...head,
    command,
    ...(args.length ? { args } : {}),
    ...(Object.keys(env).length ? { env } : {}),
    ...(cwd ? { cwd } : {}),
  }
}

const isText = (value: unknown): value is string | undefined | null =>
  value === undefined || value === null || typeof value === "string"

function isTextMap(value: unknown): value is Record<string, string> | undefined | null {
  if (value === undefined || value === null) return true
  if (typeof value !== "object" || Array.isArray(value)) return false
  return Object.values(value).every((entry) => typeof entry === "string")
}

const filled = (value: Record<string, string> | undefined | null) =>
  !!value && Object.keys(value).length > 0

/** A desktop MCP server a configuration declares, or why the build would refuse it. */
export function readServerDeclaration(value: unknown): ServerDeclaration {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error(t("mcp.read.object"))
  const { name, description, command, args, env, cwd, url, headers } = value as Record<
    string,
    unknown
  >
  if (typeof name !== "string" || !SERVER_NAME.test(name)) throw new Error(t("mcp.read.name"))
  const wrong = () => new Error(t("mcp.read.shape", { name }))
  if (!isText(description) || !isText(command) || !isText(cwd) || !isText(url)) throw wrong()
  if (!isTextMap(env) || !isTextMap(headers)) throw wrong()
  if (
    args !== undefined &&
    args !== null &&
    (!Array.isArray(args) || args.some((arg) => typeof arg !== "string"))
  )
    throw wrong()
  const list = (args ?? []) as string[]
  const runs = !!command?.trim()
  const listens = !!url?.trim()
  const stdioOnly = list.length > 0 || filled(env) || !!cwd
  // Exactly one way to reach it, with only that way's settings.
  if (runs === listens || (runs && filled(headers)) || (listens && stdioOnly))
    throw new Error(t("mcp.read.commandOrUrl", { name }))
  return {
    name,
    ...(description ? { description } : {}),
    ...(runs
      ? {
          command: command!,
          ...(list.length ? { args: list } : {}),
          ...(filled(env) ? { env: env! } : {}),
          ...(cwd ? { cwd } : {}),
        }
      : { url: url!, ...(filled(headers) ? { headers: headers! } : {}) }),
  }
}

/** The desktop MCP servers a configuration declares, each name once. */
export function readServerDeclarations(value: unknown): ServerDeclaration[] {
  if (!Array.isArray(value)) throw new Error(t("mcp.read.list"))
  const seen = new Set<string>()
  return value.map((entry) => {
    const server = readServerDeclaration(entry)
    if (seen.has(server.name)) throw new Error(t("mcp.read.twice", { name: server.name }))
    seen.add(server.name)
    return server
  })
}
