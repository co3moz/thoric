// Scalable containers: a container whose request is kept as a scale set's template, which creates
// more instances of it on demand. Each instance is a container of its own from the same image and
// settings, numbered from 1; what they share are volumes, each folder by the set's policy. Each
// desktop MCP server runs in every instance, or in instance 1 alone for all of them.
import type { ContainerItem } from "./types"

/**
 * What an instance mounts for a volume of the template. `first_writes` mounts the template's in
 * every instance, read-only in all but instance 1.
 */
export type VolumePolicy = "shared" | "new" | "first" | "first_writes"

/** The policies in the order the forms offer them. */
export const POLICIES: VolumePolicy[] = ["shared", "first_writes", "new", "first"]

/**
 * Whether a volume at `target` must stay writable in every instance, so `first_writes` is not
 * offered there: the desktop home and its configuration folders, which the session writes, as
 * `must_stay_writable` in `core/manager/src/model.rs` says.
 */
export const mustStayWritable = (target: string) =>
  target === "/home/desktop" || target.startsWith("/home/desktop/.")

/** The policies a form offers for a volume at `target`. */
export const policiesFor = (target: string) =>
  mustStayWritable(target) ? POLICIES.filter((policy) => policy !== "first_writes") : POLICIES

/**
 * Where the instances run a desktop MCP server: `each` its own, or `first`, instance 1 alone for
 * all of them, for a server that locks what the instances share.
 */
export type ServerPolicy = "each" | "first"

/** The server policies in the order the forms offer them. */
export const SERVER_POLICIES: ServerPolicy[] = ["each", "first"]

/** The server policies a set keeps of a form's: those that are not `each`. */
export const serverPolicies = (servers: Record<string, ServerPolicy>) =>
  Object.fromEntries(Object.entries(servers).filter(([, policy]) => policy !== "each"))

/** Whether two forms of server policies say the same, `each` or left out alike. */
export function samePolicies(a: Record<string, ServerPolicy>, b: Record<string, ServerPolicy>) {
  const left = serverPolicies(a)
  const right = serverPolicies(b)
  const names = new Set([...Object.keys(left), ...Object.keys(right)])
  return [...names].every((name) => left[name] === right[name])
}

/** The most instances a set may have, as the manager's `MAX_INSTANCES`. */
export const MAX_INSTANCES = 16

/** The longest set name the manager takes. */
const NAME_LIMIT = 40

/** One instance as `GET scale-sets` lists it. */
export type ScaleInstance = {
  number: number
  container_id: string
  /** `null` while its record is still being made. */
  name: string | null
  note: string
  creating: boolean
  /** One of the lowest numbers the set keeps, which an agent and the idle delete leave alone. */
  kept: boolean
}

/**
 * What agents did in one instance, as `GET scale-sets/NAME/activity` counts it from the rows the
 * activity log keeps.
 */
export type InstanceActivity = {
  instance: number
  container_id: string
  /** Every call, a repeated read's each. */
  calls: number
  /** Those that failed or were refused. */
  errors: number
  /** When the last call began, in Unix milliseconds; null without calls. */
  last_at: number | null
}

/** The instances that took calls, in the order of their numbers. */
export const activeInstances = (instances: InstanceActivity[] | undefined) =>
  (instances ?? []).filter((item) => item.calls > 0).sort((a, b) => a.instance - b.instance)

/** A volume of the template: where it is mounted, the template's volume there and its policy. */
export type ScaleSlot = {
  target: string
  /** Empty for a compose parameter given no volume. */
  volume: string
  /** The compose value it is, by the name the value goes by; `null` for a volume of the settings. */
  parameter: string | null
  readonly: boolean
  policy: VolumePolicy
  /** The compose parameter's label, when a compose names the folder. */
  label: string | null
}

export type ScaleSet = {
  name: string
  created_at: number
  image: string
  min: number
  max: number
  idle_delete_minutes: number
  /** The folders whose policy is not `shared`. */
  volumes: Record<string, VolumePolicy>
  /** The desktop servers whose policy is not `each`; a manager before them sends none. */
  servers?: Record<string, ServerPolicy>
  instances: ScaleInstance[]
  endpoint: string | null
  endpoint_path: string | null
  slots: ScaleSlot[]
  /** The set is being deleted with its instances; nothing is created or changed meanwhile. */
  deleting?: boolean
}

/** The scaling section of a form, as the fields edit it. */
export type ScalingDraft = {
  enabled: boolean
  /** Empty for the name made of the container's. */
  set: string
  min: string
  max: string
  idle: string
  volumes: Record<string, VolumePolicy>
  servers: Record<string, ServerPolicy>
}

export const scalingDraft = (set?: Partial<ScaleSet>): ScalingDraft => ({
  enabled: false,
  set: set?.name ?? "",
  min: String(set?.min ?? 1),
  max: String(set?.max ?? 4),
  idle: String(set?.idle_delete_minutes ?? 0),
  volumes: { ...(set?.volumes ?? {}) },
  servers: { ...(set?.servers ?? {}) },
})

/**
 * The set name the manager makes of a container's: lowercase letters and digits with dashes
 * between words, at most 40, or `set` when nothing is left.
 */
export function setNameOf(name: string) {
  let out = ""
  for (const char of name.toLowerCase()) {
    if (/[a-z0-9]/.test(char)) out += char
    else if (out && !out.endsWith("-")) out += "-"
  }
  out = out.slice(0, NAME_LIMIT).replace(/-+$/, "")
  return out || "set"
}

/** A set name as the manager takes it: a slug of at most 40 that neither starts nor ends with a dash. */
export const SET_NAME_PATTERN = "[a-z0-9]([a-z0-9\\-]{0,38}[a-z0-9])?"

/** A whole number from a field, or `null` for one that is not. */
const whole = (value: string) => (/^\d+$/.test(value.trim()) ? Number(value.trim()) : null)

/**
 * Why the draft's numbers cannot be sent, or `null` when they can: the maximum from 1 to 16, the
 * minimum within it, and the idle delete in whole minutes.
 */
export function scalingError(draft: ScalingDraft): "min" | "max" | "idle" | null {
  const min = whole(draft.min)
  const max = whole(draft.max)
  const idle = whole(draft.idle)
  if (max === null || max < 1 || max > MAX_INSTANCES) return "max"
  if (min === null || min > max) return "min"
  if (idle === null || idle > 10080) return "idle"
  return null
}

/**
 * The `scaling` a request sends for the draft, volume policies other than `shared` and server
 * policies other than `each` alone.
 */
export function scalingBody(draft: ScalingDraft) {
  const volumes = Object.fromEntries(
    Object.entries(draft.volumes).filter(([, policy]) => policy !== "shared")
  )
  const servers = serverPolicies(draft.servers)
  return {
    ...(draft.set.trim() ? { set: draft.set.trim() } : {}),
    min: whole(draft.min) ?? 1,
    max: whole(draft.max) ?? 4,
    idle_delete_minutes: whole(draft.idle) ?? 0,
    volumes,
    ...(Object.keys(servers).length ? { servers } : {}),
  }
}

/**
 * A set's desktop servers, each name once, as its instances declare them in the order of their
 * numbers: the first instance's record of a server, or, for one the set runs in instance 1
 * alone, that instance's, which alone reads its tools.
 */
export function setServers<T extends { container_id: string; name: string }>(
  set: ScaleSet | undefined,
  servers: T[]
): T[] {
  const instances = [...(set?.instances ?? [])].sort((a, b) => a.number - b.number)
  const out: T[] = []
  for (const instance of instances)
    for (const server of servers
      .filter((entry) => entry.container_id === instance.container_id)
      .sort((a, b) => a.name.localeCompare(b.name))) {
      if (out.some((entry) => entry.name === server.name)) continue
      if (set?.servers?.[server.name] === "first" && instance.number !== 1) continue
      out.push(server)
    }
  return out
}

/** The names of a set's desktop servers, also those it has a policy for that no instance runs now. */
export const setServerNames = (
  set: ScaleSet | undefined,
  servers: { container_id: string; name: string }[]
) =>
  [
    ...new Set([
      ...setServers(set, servers).map((server) => server.name),
      ...Object.keys(set?.servers ?? {}),
    ]),
  ].sort()

/** The folders a container's volumes are mounted at, each once: its settings', then its compose's. */
export function scalingTargets(mounts: { target: string }[]): string[] {
  return [...new Set(mounts.map((mount) => mount.target))]
}

/**
 * The containers grouped as the containers page shows them: each set with its instances in their
 * order, then every container of none. A set whose instances are all left out is left out too.
 */
export function bySet(containers: ContainerItem[], sets: ScaleSet[]) {
  const groups = sets
    .map((set) => ({
      set,
      instances: containers
        .filter((item) => item.scale?.set === set.name)
        .sort((a, b) => (a.scale?.number ?? 0) - (b.scale?.number ?? 0)),
    }))
    .filter((group) => group.instances.length > 0)
  const alone = containers.filter(
    (item) => !item.scale || !sets.some((set) => set.name === item.scale?.set)
  )
  return { groups, alone }
}

/** The instances a set has, also those being made, and how many more it may have. */
export const room = (set: ScaleSet) => Math.max(0, set.max - set.instances.length)

/** One entry of the sidebar's running containers: a container, or a scale set's running instances. */
export type RunningEntry =
  | { kind: "container"; item: ContainerItem }
  | { kind: "set"; set: string; instances: ContainerItem[] }

/**
 * The running containers as the sidebar lists them, first to last: a scale set's instances
 * together under the set, in the order of their numbers, at the place of its first instance, and
 * every other container by when it was created.
 */
export function runningEntries(running: ContainerItem[]): RunningEntry[] {
  const byAge = [...running].sort(
    (a, b) => a.created_at - b.created_at || a.spec.name.localeCompare(b.spec.name)
  )
  const entries: RunningEntry[] = []
  for (const item of byAge) {
    const set = item.scale?.set
    if (!set) {
      entries.push({ kind: "container", item })
      continue
    }
    const entry = entries.find((other) => other.kind === "set" && other.set === set)
    if (entry?.kind === "set") entry.instances.push(item)
    else entries.push({ kind: "set", set, instances: [item] })
  }
  for (const entry of entries)
    if (entry.kind === "set")
      entry.instances.sort((a, b) => (a.scale?.number ?? 0) - (b.scale?.number ?? 0))
  return entries
}

/** How many containers the entries hold. */
export const entryCount = (entries: RunningEntry[]) =>
  entries.reduce((sum, entry) => sum + (entry.kind === "set" ? entry.instances.length : 1), 0)
