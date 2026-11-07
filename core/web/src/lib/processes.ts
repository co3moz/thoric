import { api } from "./api"
import type { ContainerItem } from "./types"
import type { UsageSeries } from "./usage"

// The JSON of GET /api/containers/{id}/processes; see "Applications and windows" in docs/API.md.

/** The desktop user's ID, whose processes a signal reaches without root. */
export const DESKTOP_UID = 1001

/**
 * What a process is to the desktop: part of it (its screen, sound or session), started by Thoric
 * through `docker exec`, or anything else.
 */
export type ProcessRole = "desktop" | "thoric" | "program"

export type ProcessInfo = {
  pid: number
  /** 0 for one started through `docker exec`, whose parent is outside the container. */
  ppid: number
  uid: number
  user: string
  /** The kernel's state letter, such as R, S, D, T or Z. */
  state: string
  name: string
  command: string
  role: ProcessRole
  threads: number
  /** Resident memory, shared pages counted in each process. */
  rss_bytes: number
  /** CPU over the interval, 100 for one full core. */
  cpu_percent: number
  cpu_seconds: number
  started_at: number | null
}

export type ProcessList = {
  processes: ProcessInfo[]
  total: number
  matched: number
  cpu_percent: number
  rss_bytes: number
  /** The container's memory as its cgroup counts it. */
  memory_bytes: number | null
  memory_limit_bytes: number | null
  cpus: number
  interval_ms: number
}

export type SignalName = "term" | "kill" | "int" | "hup"

export type Signalled = {
  pid: number
  signal: SignalName
  signalled: number[]
  ended: number[]
  running: number[]
  refused: number[]
}

/** As many processes as the manager lists at once; the page sorts and searches them itself. */
export const PROCESS_LIMIT = 1000

export const processesPath = (id: string) =>
  `containers/${encodeURIComponent(id)}/processes?limit=${PROCESS_LIMIT}`

/** Signals a process, as root for one of another user, and waits for it to end. */
export const signalProcess = (
  id: string,
  process: Pick<ProcessInfo, "pid" | "uid">,
  signal: SignalName,
  tree: boolean
) =>
  api<Signalled>(`containers/${encodeURIComponent(id)}/processes/${process.pid}/kill`, {
    signal,
    tree,
    root: needsRoot(process),
  })

/** Whether a signal to the process has to go as root. */
export const needsRoot = (process: Pick<ProcessInfo, "uid">) => process.uid !== DESKTOP_UID

/** Whether the process may be signalled at all: PID 1 starts the desktop, which stops with it. */
export const signallable = (process: Pick<ProcessInfo, "pid">) => process.pid !== 1

export type SortKey = "name" | "user" | "cpu" | "memory" | "pid"
export type Sort = { key: SortKey; descending: boolean }

/** The way a column sorts first: figures the largest first, words and IDs from the start. */
export const firstDescending = (key: SortKey) => key === "cpu" || key === "memory"

/** The sort after a click on `key`'s heading: the other way round, or that column's own way. */
export const nextSort = (sort: Sort, key: SortKey): Sort =>
  sort.key === key
    ? { key, descending: !sort.descending }
    : { key, descending: firstDescending(key) }

/**
 * The order of `sort`, by the figures `figures` gives a process: its own, or in the tree those of
 * the process with its descendants, so a program and its helpers rise together.
 */
export function compareProcesses(
  sort: Sort,
  figures: (process: ProcessInfo) => { cpu: number; rss: number } = (process) => ({
    cpu: process.cpu_percent,
    rss: process.rss_bytes,
  })
) {
  return (a: ProcessInfo, b: ProcessInfo) => {
    let order = 0
    switch (sort.key) {
      case "name":
        order = a.name.localeCompare(b.name)
        break
      case "user":
        order = a.user.localeCompare(b.user)
        break
      case "cpu": {
        const [x, y] = [figures(a), figures(b)]
        order = x.cpu - y.cpu || x.rss - y.rss
        break
      }
      case "memory": {
        const [x, y] = [figures(a), figures(b)]
        order = x.rss - y.rss || x.cpu - y.cpu
        break
      }
      case "pid":
        order = a.pid - b.pid
        break
    }
    return (sort.descending ? -order : order) || a.pid - b.pid
  }
}

/** Whether the process's name, command line or user contains `query`, or its PID is it. */
export function matchesProcess(process: ProcessInfo, query: string) {
  const text = query.trim().toLowerCase()
  if (!text) return true
  return (
    String(process.pid) === text ||
    [process.name, process.command, process.user].some((value) =>
      value.toLowerCase().includes(text)
    )
  )
}

export type ProcessTotals = { cpu: number; rss: number; count: number }

/** A row of the process list, with what it and its descendants use together in the tree. */
export type ProcessRow = {
  process: ProcessInfo
  depth: number
  /** Its children among the listed processes; always 0 in the flat list. */
  children: number
  /** The process with its descendants in the tree, the process alone in the flat list. */
  totals: ProcessTotals
}

const own = (process: ProcessInfo): ProcessTotals => ({
  cpu: process.cpu_percent,
  rss: process.rss_bytes,
  count: 1,
})

/** The children of every listed process, by its PID. */
export function childrenOf(processes: ProcessInfo[]) {
  const listed = new Set(processes.map((process) => process.pid))
  const children = new Map<number, ProcessInfo[]>()
  for (const process of processes) {
    // A process whose parent is not listed, such as one started through docker exec, is a root.
    const parent = listed.has(process.ppid) && process.ppid !== process.pid ? process.ppid : 0
    const siblings = children.get(parent)
    if (siblings) siblings.push(process)
    else children.set(parent, [process])
  }
  return children
}

/** The place of each process in rows shown before, by its PID, which `held` keeps them in. */
export const rowOrder = (rows: ProcessRow[]): ReadonlyMap<number, number> =>
  new Map(rows.map((row, index) => [row.process.pid, index]))

/**
 * The rows to show: the matching processes sorted, or with `tree` every process under its parent,
 * siblings sorted by what they use with their descendants, those that match a search with the
 * processes above them and the descendants of a folded one left out. A search unfolds the tree,
 * so every match shows. With `held`, the processes keep the places it gives them while their
 * figures change, so a row does not move from under the pointer; one it has no place for goes
 * after them, sorted.
 */
export function processRows(
  processes: ProcessInfo[],
  options: {
    sort: Sort
    query: string
    tree: boolean
    folded: ReadonlySet<number>
    held?: ReadonlyMap<number, number> | null
  }
): ProcessRow[] {
  const held = options.held
  const keep =
    (compare: (a: ProcessInfo, b: ProcessInfo) => number) => (a: ProcessInfo, b: ProcessInfo) =>
      held
        ? (held.get(a.pid) ?? Infinity) - (held.get(b.pid) ?? Infinity) || compare(a, b)
        : compare(a, b)
  if (!options.tree)
    return processes
      .filter((process) => matchesProcess(process, options.query))
      .sort(keep(compareProcesses(options.sort)))
      .map((process) => ({ process, depth: 0, children: 0, totals: own(process) }))

  const children = childrenOf(processes)
  const totals = new Map<number, ProcessTotals>()
  const shown = new Set<number>()
  const searching = options.query.trim() !== ""
  // Each process once, children before their parent, so a parent adds up what is below it.
  const visit = (process: ProcessInfo, seen: Set<number>): ProcessTotals => {
    const sum = own(process)
    let matched = matchesProcess(process, options.query)
    for (const child of children.get(process.pid) ?? []) {
      if (seen.has(child.pid)) continue
      seen.add(child.pid)
      const below = visit(child, seen)
      sum.cpu += below.cpu
      sum.rss += below.rss
      sum.count += below.count
      matched ||= shown.has(child.pid)
    }
    totals.set(process.pid, sum)
    if (matched) shown.add(process.pid)
    return sum
  }
  const seen = new Set<number>()
  const roots = children.get(0) ?? []
  for (const root of roots) {
    seen.add(root.pid)
    visit(root, seen)
  }

  const compare = keep(
    compareProcesses(options.sort, (process) => totals.get(process.pid) ?? own(process))
  )
  const rows: ProcessRow[] = []
  const place = (process: ProcessInfo, depth: number) => {
    if (!shown.has(process.pid)) return
    const below = [...(children.get(process.pid) ?? [])].sort(compare)
    rows.push({ process, depth, children: below.length, totals: totals.get(process.pid)! })
    if (!searching && options.folded.has(process.pid)) return
    for (const child of below) place(child, depth + 1)
  }
  for (const root of [...roots].sort(compare)) place(root, 0)
  return rows
}

/** A running container with what its latest usage sample says, for the header's menu. */
export type RunningContainer = {
  id: string
  name: string
  /** CPU cores in use, as the usage charts count them; `null` before the first sample. */
  cpu: number | null
  memory: number | null
}

/**
 * The running containers, the busiest first and those not yet measured last by name, with their
 * latest usage sample when it is no older than five seconds.
 */
export function runningContainers(
  containers: ContainerItem[],
  usage: UsageSeries[],
  latest: number
): RunningContainer[] {
  const samples = new Map(usage.map((series) => [series.id, series.samples.at(-1)]))
  return containers
    .filter((container) => container.state === "running")
    .map((container) => {
      const sample = samples.get(container.id)
      const recent = sample && latest - sample.t <= 5000 ? sample : undefined
      return {
        id: container.id,
        name: container.spec.name,
        cpu: recent?.cpu ?? null,
        memory: recent?.memory ?? null,
      }
    })
    .sort(
      (a, b) =>
        (b.cpu ?? -1) - (a.cpu ?? -1) || a.name.localeCompare(b.name) || a.id.localeCompare(b.id)
    )
}
