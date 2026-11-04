import { formatDate, formatNumber, formatRelative, t } from "./i18n"
import type { ContainerItem, ContainerSpec, Lifecycle, OnExpiry, Settings } from "./types"

/** The policy of a container that has none, as the manager reads it: never. */
export const NEVER: Lifecycle = {
  idle_stop_minutes: 0,
  expires_at: null,
  on_expiry: "stop",
  delete_volumes: false,
}

/** The longest idle stop the manager takes, a week in minutes. */
export const MAX_IDLE_MINUTES = 10080

/** The idle stops the forms offer before a length of one's own, in minutes. */
export const IDLE_PRESETS = [15, 30, 60, 240]

/** The expiries the forms offer before a time of one's own, in minutes from when it is sent. */
export const EXPIRY_PRESETS = [60, 480, 1440, 10080]

/**
 * The lifecycle fields of the create and settings forms. `expires` is a `datetime-local` text, and
 * `expiresIn` the minutes of an expiry chosen from `EXPIRY_PRESETS`, which the manager counts from
 * when the request reaches it, so a form left open does not move it into the past.
 */
export type LifecycleDraft = {
  idle: string
  expires: string
  expiresIn?: string
  onExpiry: OnExpiry
  deleteVolumes: boolean
}

/** What a request may send of a lifecycle: the policy's fields, or an expiry in minutes. */
export type LifecycleRequest = Partial<Lifecycle> & { expires_in_minutes?: number }

const pad = (value: number) => String(value).padStart(2, "0")

/** A time in Unix seconds as a `datetime-local` field holds it, in the browser's time zone. */
export function localTime(seconds: number) {
  const date = new Date(seconds * 1000)
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

/** The Unix seconds of a `datetime-local` field's text, or `null` when it holds no time. */
export function unixTime(text: string) {
  if (!text) return null
  const millis = new Date(text).getTime()
  return Number.isNaN(millis) ? null : Math.floor(millis / 1000)
}

/**
 * A container's lifecycle as the form starts it. A new container, given `settings`, starts from
 * the installation's idle stop, as a create request that leaves it out does.
 */
export function lifecycleDraft(spec: Partial<ContainerSpec>, settings?: Settings): LifecycleDraft {
  const lifecycle = spec.lifecycle ?? {
    ...NEVER,
    idle_stop_minutes: settings?.defaults?.idle_stop_minutes ?? 0,
  }
  return {
    idle: lifecycle.idle_stop_minutes ? String(lifecycle.idle_stop_minutes) : "",
    expires: lifecycle.expires_at ? localTime(lifecycle.expires_at) : "",
    onExpiry: lifecycle.on_expiry,
    deleteVolumes: lifecycle.delete_volumes,
  }
}

/**
 * The `lifecycle` a request sends. A new container gets all of it; a change, `start` being the
 * form as it opened, only the fields the form changed, since the manager keeps the others, such
 * as a later expiry an agent set meanwhile, and nothing when it changed none.
 */
export function lifecycleBody(
  draft: LifecycleDraft,
  start?: LifecycleDraft
): { lifecycle?: LifecycleRequest } {
  const deletes = (form: LifecycleDraft) => form.onExpiry === "delete" && form.deleteVolumes
  const minutes = Number(draft.expiresIn) || 0
  // The manager takes one of the two, never both.
  const expiry: LifecycleRequest = minutes
    ? { expires_in_minutes: minutes }
    : { expires_at: unixTime(draft.expires) }
  const full: LifecycleRequest = {
    idle_stop_minutes: Number(draft.idle) || 0,
    ...expiry,
    on_expiry: draft.onExpiry,
    delete_volumes: deletes(draft),
  }
  if (!start) return { lifecycle: full }
  const changed: LifecycleRequest = {}
  if ((Number(draft.idle) || 0) !== (Number(start.idle) || 0))
    changed.idle_stop_minutes = full.idle_stop_minutes
  if (minutes || draft.expires !== start.expires) Object.assign(changed, expiry)
  if (draft.onExpiry !== start.onExpiry) changed.on_expiry = full.on_expiry
  if (deletes(draft) !== deletes(start)) changed.delete_volumes = full.delete_volumes
  return Object.keys(changed).length ? { lifecycle: changed } : {}
}

/** Minutes as a short length of time in the largest whole unit, such as 30 min, 2 hr or 1 day. */
export function minutesText(minutes: number) {
  const units = [
    { unit: "week", size: 10080 },
    { unit: "day", size: 1440 },
    { unit: "hour", size: 60 },
  ]
  const { unit, size } = units.find(({ size }) => minutes >= size && minutes % size === 0) ?? {
    unit: "minute",
    size: 1,
  }
  return formatNumber(minutes / size, { style: "unit", unit, unitDisplay: "short" })
}

/**
 * The form's lifecycle in a few words for its footer, at `now` in Unix seconds: its idle stop and
 * its expiry, or that Thoric leaves the container alone.
 */
export function lifecycleSummary(draft: LifecycleDraft, now: number) {
  const parts: string[] = []
  const idle = Number(draft.idle) || 0
  if (idle) parts.push(t("lifecycle.badge.idle", { duration: minutesText(idle) }))
  const minutes = Number(draft.expiresIn) || 0
  const at = minutes ? now + minutes * 60 : unixTime(draft.expires)
  if (at)
    parts.push(
      t(draft.onExpiry === "delete" ? "lifecycle.badge.deleted" : "lifecycle.badge.expires", {
        time: aheadText(at, now),
      })
    )
  return parts.length ? parts.join(" · ") : t("lifecycle.summary.never")
}

/** How far ahead `at` is from `now`, both in Unix seconds, such as "in 2 hr." */
export function aheadText(at: number, now: number) {
  const seconds = Math.max(0, at - now)
  if (seconds < 90 * 60) return formatRelative(Math.max(1, Math.round(seconds / 60)), "minute")
  if (seconds < 48 * 3600) return formatRelative(Math.round(seconds / 3600), "hour")
  return formatRelative(Math.round(seconds / 86400), "day")
}

/** A badge a container's card shows of its lifecycle, with the line its tooltip gives. */
export type LifecycleBadge = { kind: "idle" | "expiry"; text: string; detail: string }

/**
 * What a container's card says of its lifecycle at `now`, in Unix seconds: its idle stop, with
 * how soon it comes while a running desktop goes unused, and its expiry, which a passed one says
 * is being carried out.
 */
export function lifecycleBadges(item: ContainerItem, now: number): LifecycleBadge[] {
  const lifecycle = item.spec.lifecycle ?? NEVER
  const badges: LifecycleBadge[] = []
  const minutes = lifecycle.idle_stop_minutes
  if (minutes) {
    const duration = minutesText(minutes)
    const running = item.state === "running"
    // While something uses the desktop, the stop is a whole idle time away and does not come
    // nearer, so only an unused one counts down.
    const counting = running && item.in_use === false && item.idle_stop_at != null
    badges.push({
      kind: "idle",
      text: counting
        ? t("lifecycle.badge.idleStops", { time: aheadText(item.idle_stop_at!, now) })
        : t("lifecycle.badge.idle", { duration }),
      detail: running
        ? counting
          ? t("lifecycle.badge.idleUnused", { duration })
          : t("lifecycle.badge.idleInUse", { duration })
        : t("lifecycle.badge.idleStopped", { duration }),
    })
  }
  const at = lifecycle.expires_at
  if (at) {
    const deletes = lifecycle.on_expiry === "delete"
    const date = formatDate(at)
    badges.push({
      kind: "expiry",
      text:
        at <= now
          ? t("lifecycle.badge.expiring")
          : t(deletes ? "lifecycle.badge.deleted" : "lifecycle.badge.expires", {
              time: aheadText(at, now),
            }),
      detail: deletes
        ? t(
            lifecycle.delete_volumes
              ? "lifecycle.badge.deletedAtVolumes"
              : "lifecycle.badge.deletedAt",
            { date }
          )
        : t("lifecycle.badge.stoppedAt", { date }),
    })
  }
  return badges
}
