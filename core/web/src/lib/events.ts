import type { DownloadProgress, HumanRequest, Job } from "./types"

/**
 * The manager's event stream, `/api/events`: what happens as it happens, so the dashboard hears
 * that a job ended, or that Docker changed a list by itself, without polling for it. Lists still
 * poll, more slowly while the stream is up, and as before while it is down, so the stream speeds
 * things up and is never the only way.
 */

/** A job as its event describes it: its record without the log, the result and the stages. */
export type JobEvent = Pick<
  Job,
  "id" | "kind" | "resource" | "name" | "status" | "created_at" | "finished_at"
> & { error: string | null }

export type ServerEvent =
  /** `docker_events`: whether Docker's own events reach the stream; older managers leave it out. */
  | { type: "ready"; docker_events?: boolean }
  | { type: "resync" }
  | { type: "job"; job: JobEvent }
  | { type: "job-progress"; id: string; progress: DownloadProgress | null }
  /** An agent asks for a person, or someone answered it. */
  | { type: "human-request" | "human-request-answered"; request: HumanRequest }
  /** Docker changed these lists by itself, such as a desktop that exited. */
  | { type: "changed"; lists: string[] }
  /** Docker's own events began or stopped reaching the stream. */
  | { type: "docker-events"; live: boolean }

/** Every event type the dashboard handles, which `docs/API.md` describes. */
export const EVENT_TYPES = [
  "ready",
  "resync",
  "job",
  "job-progress",
  "human-request",
  "human-request-answered",
  "changed",
  "docker-events",
] as const

/** An event's data, or `null` for one this page does not know, such as from a newer manager. */
export function parseEvent(data: string): ServerEvent | null {
  try {
    const event = JSON.parse(data) as { type?: unknown }
    return EVENT_TYPES.includes(event?.type as (typeof EVENT_TYPES)[number])
      ? (event as ServerEvent)
      : null
  } catch {
    return null
  }
}

const finished = (job: Pick<Job, "status">) =>
  job.status === "completed" || job.status === "failed" || job.status === "interrupted"

/** What else a job changes when it ends, beyond the jobs themselves. */
const changes: Record<string, string[]> = {
  // A creation may make volumes for its parameters and an endpoint; a deletion may remove them.
  create: ["containers", "volumes", "mcp"],
  delete: ["containers", "volumes", "mcp"],
  snapshot: ["containers", "images"],
  // A duplicate makes a container, the snapshot it starts from, copies of volumes and an endpoint.
  duplicate: ["containers", "images", "volumes", "mcp"],
  build: ["images"],
  "image-pull": ["images"],
  "source-build": ["images"],
  "image-import": ["images"],
  "volume-import": ["volumes"],
  "volume-copy": ["volumes"],
  // A restore creates containers, images, volumes and endpoints from the backup.
  restore: ["containers", "images", "volumes", "mcp"],
  // Archives, downloads and scripts leave the lists as they were.
  "image-export": [],
  "volume-export": [],
  "volume-extract": [],
  "folder-download": [],
  exec: [],
}

/**
 * The query keys a job event makes stale: the jobs, and once it ended, the lists its kind
 * changes, or for a kind this page does not know, the list its resource belongs to.
 */
export function staleKeys(job: Pick<Job, "kind" | "resource" | "status">) {
  const keys = ["jobs"]
  if (!finished(job)) return keys
  const known = changes[job.kind]
  if (known) return [...keys, ...known]
  const kind = job.resource?.split(":")[0]
  const list = { container: "containers", image: "images", volume: "volumes" }[kind ?? ""]
  return list ? [...keys, list] : keys
}

/** Kinds whose outcome the page that started them shows already, or that tell nothing new. */
const quiet = new Set([
  // A script's log opens when it starts, and agents run many.
  "exec",
  // The browser shows a download; its job only records it.
  "folder-download",
  "image-export",
  "volume-export",
  // The page reloads into the new manager and says so itself.
  "update",
])

/** Kinds that make something, whose end is worth telling however quick it was. */
const makes = new Set([
  "create",
  "duplicate",
  "build",
  "snapshot",
  "image-pull",
  "source-build",
  "image-import",
  "volume-import",
  "volume-copy",
  "volume-extract",
])

/** A job that took this many seconds is told when it ends, whatever it did. */
export const LONG_JOB = 10

/** Whether the end of a job is worth a toast, and of which kind. */
export function jobNotice(
  job: Pick<Job, "kind" | "status" | "created_at" | "finished_at">
): "success" | "error" | null {
  if (quiet.has(job.kind)) return null
  if (job.status === "failed" || job.status === "interrupted") return "error"
  if (job.status !== "completed") return null
  const took = (job.finished_at ?? job.created_at) - job.created_at
  return makes.has(job.kind) || took >= LONG_JOB ? "success" : null
}

/**
 * The jobs that ended between two reads of the job list, such as while the stream was down: those
 * the first read had queued or running and the second has finished.
 */
export function endedJobs(before: Job[], after: Job[]): Job[] {
  const active = new Set(before.filter((job) => !finished(job)).map((job) => job.id))
  return after.filter((job) => active.has(job.id) && finished(job))
}

/** A list of jobs with one job's measured progress replaced. */
export function withProgress<T extends Pick<Job, "id" | "progress">>(
  jobs: T[] | undefined,
  id: string,
  progress: DownloadProgress | null
) {
  if (!jobs?.some((job) => job.id === id)) return jobs
  return jobs.map((job) => (job.id === id ? { ...job, progress } : job))
}

/** How long to wait before the `attempt`th reconnection: one second, doubling up to 30. */
export function retryDelay(attempt: number, random = Math.random) {
  const base = Math.min(30_000, 1000 * 2 ** Math.min(attempt, 5))
  // Spread out, so pages that lost the manager together do not all return at once.
  return Math.round(base * (0.8 + 0.4 * random()))
}

/** What an open stream reports to, and the stream itself, so tests can stand in for `EventSource`. */
export type Handlers = { message(data: string): void; error(): void }
export type Source = { close(): void }

/** Opens the manager's stream in the browser. */
export function openEventSource({ message, error }: Handlers): Source {
  const source = new EventSource("/api/events")
  source.onmessage = (event: MessageEvent<string>) => message(event.data)
  source.onerror = error
  return source
}

/** Whether the page is in the background, and a way to hear when that changes. */
export type Visibility = { hidden(): boolean; listen(change: () => void): () => void }

const pageVisibility: Visibility = {
  hidden: () => document.visibilityState === "hidden",
  listen: (change) => {
    document.addEventListener("visibilitychange", change)
    return () => document.removeEventListener("visibilitychange", change)
  },
}

export type StreamOptions = {
  /** Opens the stream: `openEventSource` in the browser. */
  open: (handlers: Handlers) => Source
  onEvent: (event: ServerEvent) => void
  /**
   * The stream is up again: `resumed` after a gap, when the page should read everything again.
   * `dockerEvents` is whether Docker's own events reach it, when the manager said so.
   */
  onReady: (resumed: boolean, dockerEvents?: boolean) => void
  /** Whether the stream is up, as polling and other views want to know. */
  onLive: (live: boolean) => void
  /**
   * Asked before reconnecting after an error: whether the session still holds. `false` stops the
   * stream for good, as the page is signing out; a thrown error retries later.
   */
  check: () => Promise<boolean>
  /** How long a page in the background keeps its stream. */
  hiddenFor?: number
  visibility?: Visibility
  delay?: (attempt: number) => number
}

/** How long a page in the background keeps its stream before it lets the connection go. */
export const HIDDEN_GRACE = 60_000

/**
 * Keep one stream open while the page is in front: reconnect after an error with a growing delay,
 * once the session is known to hold, and let it go a minute after the page went to the
 * background, since browsers allow only six connections to one HTTP/1.1 host across all tabs.
 * Returns what stops it.
 */
export function keepStream(options: StreamOptions) {
  const visibility = options.visibility ?? pageVisibility
  const delay = options.delay ?? retryDelay
  let source: Source | null = null
  let live = false
  let attempt = 0
  // Whether events may have been missed since the page last read its lists.
  let gap = false
  let stopped = false
  let retry: ReturnType<typeof setTimeout> | undefined
  let release: ReturnType<typeof setTimeout> | undefined

  const setLive = (value: boolean) => {
    if (live === value) return
    live = value
    options.onLive(value)
  }
  const drop = () => {
    source?.close()
    source = null
    gap = true
    setLive(false)
  }
  const connect = () => {
    if (stopped || source) return
    const current: Source = options.open({
      message: (data) => {
        if (source !== current) return
        const event = parseEvent(data)
        if (!event) return
        if (event.type === "ready") {
          attempt = 0
          setLive(true)
          options.onReady(gap, event.docker_events === true)
          gap = false
        } else if (event.type === "resync") {
          options.onReady(true)
        } else {
          options.onEvent(event)
        }
      },
      // EventSource retries by itself at a fixed pace and cannot tell a 401 from a lost
      // connection; the stream is closed and opened again here instead.
      error: () => {
        if (source !== current) return
        drop()
        schedule()
      },
    })
    source = current
  }
  const schedule = () => {
    if (stopped || retry !== undefined) return
    retry = setTimeout(async () => {
      if (stopped) return
      let holds: boolean
      try {
        holds = await options.check()
      } catch {
        retry = undefined
        attempt += 1
        schedule()
        return
      }
      retry = undefined
      if (!holds) {
        stop()
        return
      }
      // A page in the background reconnects when it comes back.
      if (!visibility.hidden()) connect()
    }, delay(attempt))
  }
  const changed = () => {
    clearTimeout(release)
    release = undefined
    if (visibility.hidden()) {
      if (source) release = setTimeout(drop, options.hiddenFor ?? HIDDEN_GRACE)
    } else if (!source && retry === undefined) {
      attempt = 0
      connect()
    }
  }
  const stop = () => {
    stopped = true
    clearTimeout(retry)
    clearTimeout(release)
    unlisten()
    source?.close()
    source = null
    setLive(false)
  }

  const unlisten = visibility.listen(changed)
  if (!visibility.hidden()) connect()
  return stop
}

let streaming = false
let docker = false
const listeners = new Set<() => void>()

/** Whether the event stream is up, which lets lists poll less often. */
export const streamLive = () => streaming

/**
 * Whether Docker's own events reach the page, which lets what Docker changes by itself
 * (containers, images, volumes and the system summary) poll less often.
 */
export const dockerEventsLive = () => streaming && docker

export function setDockerEvents(live: boolean) {
  docker = live
}

export function setStreamLive(live: boolean) {
  streaming = live
  listeners.forEach((listener) => listener())
}

export function onStreamLive(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}
