import { describe, expect, test } from "bun:test"
import { readFileSync } from "node:fs"

import {
  EVENT_TYPES,
  dockerEventsLive,
  endedJobs,
  jobNotice,
  keepStream,
  parseEvent,
  retryDelay,
  setDockerEvents,
  setStreamLive,
  staleKeys,
  withProgress,
  type Handlers,
  type ServerEvent,
} from "../src/lib/events"
import type { Job } from "../src/lib/types"

const job = (id: string, status: Job["status"], extra: Partial<Job> = {}): Job => ({
  id,
  kind: "build",
  status,
  created_at: 100,
  finished_at: status === "queued" || status === "running" ? null : 105,
  log: "",
  ...extra,
})

describe("reading events", () => {
  test("knows the types the manager sends and passes over others", () => {
    expect(parseEvent('{"type":"ready"}')).toEqual({ type: "ready" })
    expect(parseEvent('{"type":"job-progress","id":"a","progress":null}')).toEqual({
      type: "job-progress",
      id: "a",
      progress: null,
    })
    expect(parseEvent('{"type":"changed","lists":["containers"]}')).toEqual({
      type: "changed",
      lists: ["containers"],
    })
    expect(parseEvent('{"type":"docker-events","live":false}')).toEqual({
      type: "docker-events",
      live: false,
    })
    expect(parseEvent('{"type":"from-a-newer-manager"}')).toBeNull()
    expect(parseEvent("not json")).toBeNull()
    expect(parseEvent("null")).toBeNull()
  })

  test("a job's change makes the job list stale, and its end the lists its kind changes", () => {
    expect(staleKeys({ kind: "create", resource: "container:a", status: "running" })).toEqual([
      "jobs",
    ])
    expect(staleKeys({ kind: "create", resource: "container:a", status: "completed" })).toEqual([
      "jobs",
      "containers",
      "volumes",
      "mcp",
    ])
    // A duplicate's container starts from a snapshot, with copied volumes and an endpoint.
    expect(staleKeys({ kind: "duplicate", resource: "container:b", status: "failed" })).toEqual([
      "jobs",
      "containers",
      "images",
      "volumes",
      "mcp",
    ])
    expect(staleKeys({ kind: "build", resource: "image:a", status: "failed" })).toEqual([
      "jobs",
      "images",
    ])
    expect(staleKeys({ kind: "exec", resource: "container:a", status: "completed" })).toEqual([
      "jobs",
    ])
    // A kind this page does not know yet refreshes the list its resource belongs to.
    expect(staleKeys({ kind: "later", resource: "volume:v", status: "completed" })).toEqual([
      "jobs",
      "volumes",
    ])
    expect(staleKeys({ kind: "later", resource: "update", status: "completed" })).toEqual(["jobs"])
    // A restore works on the installation, and creates every kind of thing.
    expect(staleKeys({ kind: "restore", resource: "installation", status: "completed" })).toEqual([
      "jobs",
      "containers",
      "images",
      "volumes",
      "mcp",
    ])
  })

  test("tells failures, what was made and long work, and keeps scripts and downloads quiet", () => {
    expect(jobNotice(job("a", "failed", { kind: "start" }))).toBe("error")
    expect(jobNotice(job("a", "interrupted", { kind: "delete" }))).toBe("error")
    expect(jobNotice(job("a", "completed", { kind: "build" }))).toBe("success")
    // A quick start is seen on the page that asked for it; a long one may not be.
    expect(jobNotice(job("a", "completed", { kind: "start", finished_at: 102 }))).toBeNull()
    expect(jobNotice(job("a", "completed", { kind: "start", finished_at: 130 }))).toBe("success")
    for (const kind of ["exec", "folder-download", "image-export", "volume-export", "update"]) {
      expect(jobNotice(job("a", "failed", { kind }))).toBeNull()
      expect(jobNotice(job("a", "completed", { kind, finished_at: 1000 }))).toBeNull()
    }
    expect(jobNotice(job("a", "running"))).toBeNull()
  })

  test("finds the jobs that ended between two reads of the list", () => {
    const before = [job("a", "running"), job("b", "queued"), job("c", "completed")]
    const after = [
      job("d", "completed"),
      job("a", "failed"),
      job("b", "running"),
      job("c", "completed"),
    ]
    expect(endedJobs(before, after).map((ended) => ended.id)).toEqual(["a"])
  })

  test("replaces one job's progress and leaves a list without it alone", () => {
    const jobs = [job("a", "running"), job("b", "running")]
    const next = withProgress(jobs, "b", { current: 5, total: 10 })
    expect(next?.[1].progress).toEqual({ current: 5, total: 10 })
    expect(next?.[0]).toBe(jobs[0])
    expect(withProgress(jobs, "missing", null)).toBe(jobs)
    expect(withProgress(undefined, "a", null)).toBeUndefined()
  })

  test("Docker's own events slow the lists only while the stream that carries them is up", () => {
    expect(dockerEventsLive()).toBe(false)
    setDockerEvents(true)
    expect(dockerEventsLive()).toBe(false)
    setStreamLive(true)
    expect(dockerEventsLive()).toBe(true)
    setDockerEvents(false)
    expect(dockerEventsLive()).toBe(false)
    setDockerEvents(true)
    setStreamLive(false)
    expect(dockerEventsLive()).toBe(false)
    setDockerEvents(false)
  })

  test("waits longer after each failed reconnection, up to half a minute", () => {
    const middle = () => 0.5
    expect([0, 1, 2, 3, 4, 5, 9].map((attempt) => retryDelay(attempt, middle))).toEqual([
      1000, 2000, 4000, 8000, 16000, 30000, 30000,
    ])
    expect(retryDelay(0, () => 0)).toBe(800)
    expect(retryDelay(0, () => 1)).toBe(1200)
  })
})

/** A stand-in for the manager's stream and the page's visibility. */
function harness({ hidden = false, holds = async () => true as boolean } = {}) {
  const opened: { handlers: Handlers; closed: boolean }[] = []
  const seen: string[] = []
  const docker: (boolean | undefined)[] = []
  let change = () => {}
  const state = { hidden }
  const stop = keepStream({
    open: (handlers) => {
      const entry = { handlers, closed: false }
      opened.push(entry)
      return { close: () => void (entry.closed = true) }
    },
    onEvent: (event: ServerEvent) => seen.push(event.type),
    onReady: (resumed, dockerEvents) => {
      seen.push(resumed ? "resumed" : "ready")
      docker.push(dockerEvents)
    },
    onLive: (live) => seen.push(live ? "live" : "down"),
    check: holds,
    hiddenFor: 0,
    delay: () => 0,
    visibility: {
      hidden: () => state.hidden,
      listen: (listener) => {
        change = listener
        return () => (change = () => {})
      },
    },
  })
  return {
    opened,
    seen,
    docker,
    stop,
    last: () => opened[opened.length - 1],
    send: (data: object) => opened[opened.length - 1].handlers.message(JSON.stringify(data)),
    setHidden: (value: boolean) => {
      state.hidden = value
      change()
    },
  }
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 5))

describe("keeping the stream", () => {
  test("is live once the manager says it is ready and passes events on", () => {
    const stream = harness()
    expect(stream.opened.length).toBe(1)
    stream.send({ type: "ready" })
    stream.send({ type: "job", job: { id: "a" } })
    stream.send({ type: "unknown" })
    expect(stream.seen).toEqual(["live", "ready", "job"])
    stream.stop()
    expect(stream.last().closed).toBe(true)
    expect(stream.seen.at(-1)).toBe("down")
  })

  test("passes on whether Docker's own events reach the stream", () => {
    const stream = harness()
    stream.send({ type: "ready", docker_events: true })
    // A page that fell behind does not learn it again, and an older manager does not say.
    stream.send({ type: "resync" })
    stream.send({ type: "docker-events", live: false })
    stream.last().handlers.error()
    expect(stream.docker).toEqual([true, undefined])
    expect(stream.seen).toEqual(["live", "ready", "resumed", "docker-events", "down"])
    const older = harness()
    older.send({ type: "ready" })
    expect(older.docker).toEqual([false])
    stream.stop()
    older.stop()
  })

  test("reconnects after an error once the session holds, and says the page missed events", async () => {
    const stream = harness()
    stream.send({ type: "ready" })
    stream.last().handlers.error()
    expect(stream.opened[0].closed).toBe(true)
    expect(stream.seen.at(-1)).toBe("down")
    await settle()
    expect(stream.opened.length).toBe(2)
    stream.send({ type: "ready" })
    expect(stream.seen.slice(-2)).toEqual(["live", "resumed"])
    // A page that fell behind reads everything again too.
    stream.send({ type: "resync" })
    expect(stream.seen.at(-1)).toBe("resumed")
    stream.stop()
  })

  test("stops for good when the session ended, and keeps trying while the manager is away", async () => {
    let answers = 0
    const signedOut = harness({
      holds: async () => {
        answers += 1
        if (answers < 3) throw new Error("away")
        return false
      },
    })
    signedOut.last().handlers.error()
    for (let wait = 0; wait < 10 && answers < 3; wait++) await settle()
    await settle()
    expect(answers).toBe(3)
    expect(signedOut.opened.length).toBe(1)
  })

  test("lets the connection go in the background and comes back with the page", async () => {
    const stream = harness()
    stream.send({ type: "ready" })
    stream.setHidden(true)
    await settle()
    expect(stream.last().closed).toBe(true)
    expect(stream.seen.at(-1)).toBe("down")
    stream.setHidden(false)
    expect(stream.opened.length).toBe(2)
    stream.send({ type: "ready" })
    expect(stream.seen.at(-1)).toBe("resumed")
    stream.stop()
  })

  test("a page opened in the background connects when it is shown", () => {
    const stream = harness({ hidden: true })
    expect(stream.opened.length).toBe(0)
    stream.setHidden(false)
    expect(stream.opened.length).toBe(1)
    stream.stop()
  })

  test("ignores what a replaced stream still reports", async () => {
    const stream = harness()
    const first = stream.last()
    first.handlers.error()
    await settle()
    first.handlers.message(JSON.stringify({ type: "job", job: { id: "late" } }))
    first.handlers.error()
    expect(stream.seen).not.toContain("job")
    expect(stream.opened.length).toBe(2)
    stream.stop()
  })
})

test("every event the page knows is one the manager sends and the API guide describes", () => {
  const sources = ["../../manager/src/events.rs", "../../manager/src/human.rs"]
    .map((path) => readFileSync(new URL(path, import.meta.url), "utf8"))
    .join("\n")
  const guide = readFileSync(new URL("../../../docs/API.md", import.meta.url), "utf8")
  const routes = readFileSync(new URL("../../manager/src/main.rs", import.meta.url), "utf8")
  expect(routes).toContain('"/api/events"')
  for (const type of EVENT_TYPES) {
    expect(sources).toMatch(new RegExp(`"type":\\s*"${type}"`))
    expect(guide).toContain(`\`${type}\``)
  }
})
