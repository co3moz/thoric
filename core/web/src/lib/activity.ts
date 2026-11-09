// The agent activity log: what GET /api/activity returns and how the Activity page shows it. See
// "Agent activity" in docs/API.md and docs/MCP.md.
import { duration } from "./format"
import { formatDate, formatNumber, formatRelative, has, plural, t } from "./i18n"

export type ActivityKind = "read" | "change" | "destructive"
export type ActivityOutcome = "ok" | "error" | "refused"

/** One recorded call; `endpoint` is null for Thoric MCP itself at `/mcp`. */
export type ActivityEntry = {
  id: number
  /** When the call began, in Unix milliseconds. */
  at: number
  endpoint: string | null
  credential: string
  client: string | null
  /** `tool`, `server` for a desktop server's tool, or `transfer` for a used link. */
  source: string
  tool: string
  server: string | null
  container_id: string | null
  container_name: string | null
  kind: ActivityKind
  arguments: Record<string, unknown>
  summary: string
  outcome: ActivityOutcome
  error: string | null
  /** How long the call took; for a repeated read, the longest of its calls. */
  duration_ms: number
  job_id: string | null
  /** How many more calls the row stands for: reads that repeated its call one after another. */
  repeats?: number
  /** When the last of those began, in Unix milliseconds; null for a row of one call. */
  last_at?: number | null
}

export type ActivityPage = { items: ActivityEntry[]; next: number | null; kept: number }

/** What the page narrows the log to. `caller` is `administrator` or `endpoint:NAME`. */
export type ActivityFilters = {
  container?: string
  caller?: string
  tool?: string
  errors?: boolean
  destructive?: boolean
}

export const ADMINISTRATOR = "administrator"
/** The calls the page asks for at a time. */
export const PAGE_SIZE = 50

/** A caller filter for an endpoint, or for `/mcp` without one. */
export const callerFilter = (endpoint: string | null) =>
  endpoint === null ? ADMINISTRATOR : `endpoint:${endpoint}`

/** The endpoint a caller filter names, if it names one. */
export const filterEndpoint = (caller: string | undefined) =>
  caller?.startsWith("endpoint:") ? caller.slice("endpoint:".length) : undefined

/** The API path of a page of the log, older than `before` when given. */
export function activityPath(filters: ActivityFilters, before?: number, limit = PAGE_SIZE) {
  const query = new URLSearchParams()
  if (filters.container) query.set("container", filters.container)
  const endpoint = filterEndpoint(filters.caller)
  if (filters.caller === ADMINISTRATOR) query.set("caller", ADMINISTRATOR)
  else if (endpoint) query.set("endpoint", endpoint)
  const tool = filters.tool?.trim()
  if (tool) query.set("tool", tool)
  if (filters.destructive) query.set("kind", "destructive")
  if (filters.errors) query.set("outcome", "error,refused")
  if (before !== undefined) query.set("before", String(before))
  query.set("limit", String(limit))
  return `activity?${query}`
}

/** The formats the log downloads in. */
export type ExportFormat = "csv" | "jsonl"

/** The API path of the log as a file, narrowed as the page is. */
export function exportPath(filters: ActivityFilters, format: ExportFormat) {
  const query = new URLSearchParams(activityPath(filters).slice("activity?".length))
  query.delete("limit")
  query.set("format", format)
  return `activity/export?${query}`
}

/** How many calls a row stands for: its own and the reads that repeated it. */
export const callCount = (entry: Pick<ActivityEntry, "repeats">) => 1 + (entry.repeats ?? 0)

export const filtersActive = (filters: ActivityFilters) =>
  !!(
    filters.container ||
    filters.caller ||
    filters.tool?.trim() ||
    filters.errors ||
    filters.destructive
  )

/** The theme color of a kind: destructive calls apart, changes in the edit color, reads quiet. */
export const kindColor: Record<ActivityKind, string> = {
  destructive: "text-destructive",
  change: "text-edit",
  read: "text-muted-foreground",
}

/** A kind, outcome or credential in words; one this release does not know as the manager wrote it. */
export function activityLabel(group: "kind" | "outcome" | "credential", value: string) {
  const key = `activity.${group}.${value}`
  return has(key) ? t(key) : value
}

/** The tool as the list names it: a desktop server's tool after its server. */
export const toolLabel = (entry: Pick<ActivityEntry, "tool" | "server">) =>
  entry.server ? `${entry.server} / ${entry.tool}` : entry.tool

/** Who called: Thoric MCP itself, or the endpoint by its name. */
export const callerLabel = (entry: Pick<ActivityEntry, "endpoint">) =>
  entry.endpoint ?? t("activity.caller.administrator")

/** An argument the manager kept only as its type and size. */
export type Masked = {
  $masked: "string" | "number" | "boolean" | "array" | "object"
  length?: number
  items?: number
  keys?: number
}
/** A text the manager cut, with its first part and its whole length. */
export type Truncated = { $truncated: string; length: number }

const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)
export const isMasked = (value: unknown): value is Masked => object(value) && "$masked" in value
export const isTruncated = (value: unknown): value is Truncated =>
  object(value) && typeof value.$truncated === "string"

/** What a masked value was, in words, such as "12 characters hidden". */
export function maskedLabel(value: Masked) {
  switch (value.$masked) {
    case "string":
      return plural(value.length ?? 0, {
        one: "activity.masked.string.one",
        other: "activity.masked.string.other",
      })
    case "array":
      return plural(value.items ?? 0, {
        one: "activity.masked.array.one",
        other: "activity.masked.array.other",
      })
    case "object":
      return plural(value.keys ?? 0, {
        one: "activity.masked.object.one",
        other: "activity.masked.object.other",
      })
    case "number":
      return t("activity.masked.number")
    default:
      return t("activity.masked.value")
  }
}

/**
 * An argument as the detail panel shows it: a text as it is, anything else as JSON in which a
 * masked value reads as what it was and a cut text ends in an ellipsis.
 */
export function argumentText(value: unknown): string {
  if (typeof value === "string") return value
  if (isTruncated(value)) return `${value.$truncated}…`
  if (isMasked(value)) return `‹${maskedLabel(value)}›`
  return JSON.stringify(
    value,
    (_key, item: unknown) =>
      isMasked(item) ? `‹${maskedLabel(item)}›` : isTruncated(item) ? `${item.$truncated}…` : item,
    2
  )
}

/**
 * A list of desktop or gamepad input actions, one line each: the action's type and its other
 * fields, a hidden key of a chord as a dot. Anything else is not such a list, and gets `null`.
 */
export function actionLines(value: unknown) {
  if (
    !Array.isArray(value) ||
    !value.length ||
    !value.every((item) => object(item) && typeof item.type === "string")
  )
    return null
  return (value as Record<string, unknown>[]).map((action) => ({
    type: action.type as string,
    fields: Object.entries(action)
      .filter(([key]) => key !== "type")
      .map(([key, field]): [string, string] => [
        key,
        Array.isArray(field)
          ? field.map((item) => (isMasked(item) ? "•" : argumentText(item))).join(" + ")
          : argumentText(field),
      ]),
  }))
}

/** The arguments in the order the panel lists them: by name, the count of left out ones last. */
export function argumentRows(args: Record<string, unknown>) {
  return Object.entries(args)
    .filter(([key]) => key !== "$more")
    .sort(([a], [b]) => a.localeCompare(b))
}

/** How long ago `at` was, in milliseconds since the epoch, as of `now`. */
export function relativeTime(at: number, now: number) {
  const seconds = Math.round((at - now) / 1000)
  const abs = Math.abs(seconds)
  if (abs < 45) return formatRelative(0, "second")
  if (abs < 45 * 60) return formatRelative(Math.round(seconds / 60), "minute")
  if (abs < 22 * 3600) return formatRelative(Math.round(seconds / 3600), "hour")
  if (abs < 6 * 86400) return formatRelative(Math.round(seconds / 86400), "day")
  return formatDate(at / 1000, { month: "short", day: "numeric" })
}

/** The exact moment, for a tooltip and the detail panel. */
export const exactTime = (at: number) =>
  formatDate(at / 1000, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  })

/** How long a call took: milliseconds under a second, then seconds, then minutes. */
export function callDuration(ms: number) {
  if (ms < 1000) return t("activity.duration.ms", { ms: formatNumber(Math.max(0, ms)) })
  if (ms < 60_000)
    return t("activity.duration.seconds", {
      seconds: formatNumber(ms / 1000, { maximumFractionDigits: 1 }),
    })
  return duration(Math.round(ms / 1000))
}

const dayKey = (date: Date) =>
  `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`

/** The calls by the local day they began on, newest first as they come, each day named. */
export function dayGroups<T extends Pick<ActivityEntry, "at">>(entries: T[], now = new Date()) {
  const today = dayKey(now)
  const yesterday = dayKey(new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1))
  const groups: { key: string; label: string; entries: T[] }[] = []
  for (const entry of entries) {
    const key = dayKey(new Date(entry.at))
    let group = groups[groups.length - 1]
    if (group?.key !== key) {
      const label =
        key === today
          ? t("activity.today")
          : key === yesterday
            ? t("activity.yesterday")
            : formatDate(entry.at / 1000, {
                weekday: "long",
                month: "long",
                day: "numeric",
                ...(key.slice(0, 4) === today.slice(0, 4) ? {} : { year: "numeric" }),
              })
      group = { key, label, entries: [] }
      groups.push(group)
    }
    group.entries.push(entry)
  }
  return groups
}
