import * as React from "react"
import { toast } from "sonner"

import { api, sessionExpired } from "./api"
import { openDialog, openedDialog, whenDiscarded } from "./dialogs"
import {
  endedJobs,
  jobNotice,
  keepStream,
  openEventSource,
  setDockerEvents,
  setStreamLive,
  staleKeys,
  withProgress,
  type ServerEvent,
} from "./events"
import { jobName, jobTarget } from "./format"
import { HUMAN_REQUESTS, newRequests, withRequest } from "./human"
import { t } from "./i18n"
import { isScript, OTHER_JOBS } from "./job-history"
import { queryClient } from "./queries"
import { navigate, parseRoute } from "./route"
import type { ContainerItem, HumanRequest, ImageItem, Job, Session } from "./types"

/** Lists to read again, gathered for a moment so a burst of events reads each once. */
let stale = new Set<string>()
let flush: ReturnType<typeof setTimeout> | undefined

function refresh(keys: string[]) {
  keys.forEach((key) => stale.add(key))
  flush ??= setTimeout(() => {
    const keys = stale
    stale = new Set()
    flush = undefined
    for (const key of keys)
      void queryClient.invalidateQueries(
        // The MCP pages keep several lists under one key.
        { queryKey: [key], exact: key !== "mcp" },
        { cancelRefetch: false }
      )
  }, 100)
}

/** Jobs already told about, so a job heard of twice, by its event and by a list, is told once. */
const told = new Set<string>()
/** Why each failed job failed, from its event; lists of jobs carry no error. */
const errors = new Map<string, string>()

/** Tell that a job ended, when that is worth a toast and its log is not open already. */
function announce(
  job: Pick<Job, "id" | "kind" | "resource" | "name" | "status" | "created_at" | "finished_at">
) {
  if (told.has(job.id)) return
  if (told.size > 500) told.clear()
  told.add(job.id)
  const notice = jobNotice(job)
  const open = openedDialog()
  if (!notice || (open?.kind === "job" && open.id === job.id)) return
  const containers = queryClient.getQueryData<ContainerItem[]>(["containers"]) ?? []
  const images = queryClient.getQueryData<ImageItem[]>(["images"]) ?? []
  const target =
    job.name ||
    jobTarget(
      job,
      new Map(containers.map((item) => [item.id, item.spec.name])),
      new Map(images.map((image) => [image.id, image.name]))
    )
  const action = jobName(job.kind)
  const error = errors.get(job.id)
  const options = {
    id: `job-${job.id}`,
    description: error ? [target, error].filter(Boolean).join(": ") : target || undefined,
    action: {
      label: t("jobs.viewLog"),
      onClick: () => whenDiscarded(() => openDialog({ kind: "job", id: job.id })),
    },
  }
  if (notice === "success") toast.success(t("events.jobCompleted", { action }), options)
  else if (job.status === "interrupted")
    toast.error(t("events.jobInterrupted", { action }), options)
  else toast.error(t("events.jobFailed", { action }), options)
}

/** Requests for a person already told about. */
const asked = new Set<string>()

/** Tell that an agent asks for a person, unless that desktop is open here with its strip. */
function announceRequest(request: HumanRequest) {
  if (asked.has(request.id)) return
  if (asked.size > 500) asked.clear()
  asked.add(request.id)
  const route = parseRoute(location.hash)
  if (route.page === "desktop" && route.id === request.container_id) return
  toast.warning(t("human.asks", { name: request.container_name }), {
    id: `human-${request.id}`,
    description: request.message,
    duration: 15_000,
    action: {
      label: t("human.openDesktop"),
      onClick: () => navigate({ page: "desktop", id: request.container_id }),
    },
  })
}

function handle(event: ServerEvent) {
  if (event.type === "job") {
    const { job } = event
    refresh(isScript(job) ? staleKeys(job) : [...staleKeys(job), OTHER_JOBS])
    void queryClient.invalidateQueries(
      { queryKey: ["jobs", job.id], exact: true },
      { cancelRefetch: false }
    )
    if (job.error) errors.set(job.id, job.error)
    if (errors.size > 100) errors.delete(errors.keys().next().value!)
    if (job.finished_at !== null) announce(job)
  } else if (event.type === "job-progress") {
    for (const key of ["jobs", OTHER_JOBS])
      queryClient.setQueryData<Job[]>([key], (jobs) => withProgress(jobs, event.id, event.progress))
    queryClient.setQueryData<Job>(
      ["jobs", event.id],
      (job) => job && { ...job, progress: event.progress }
    )
  } else if (event.type === "changed") {
    // The system summary counts Docker's containers too.
    refresh(event.lists.includes("containers") ? [...event.lists, "system"] : event.lists)
  } else if (event.type === "docker-events") {
    // Back after a gap whose events are lost, or gone: either way the lists are read now, at
    // the pace that now suits them.
    setDockerEvents(event.live)
    refresh(watched)
  } else if (event.type === "human-request" || event.type === "human-request-answered") {
    const { request } = event
    queryClient.setQueryData<HumanRequest[]>(HUMAN_REQUESTS, (list) => withRequest(list, request))
    if (event.type === "human-request") announceRequest(request)
    else toast.dismiss(`human-${request.id}`)
  }
}

/** Whether the session still holds, before the stream is opened again after an error. */
async function sessionHolds() {
  const session = await api<Session>("session")
  if (session.auth_enabled && !session.authenticated) {
    sessionExpired()
    return false
  }
  return true
}

/** The lists the stream keeps current, which poll slowly while it is up. */
const streamed = ["jobs", OTHER_JOBS, HUMAN_REQUESTS[0]]
/** The lists Docker's own events keep current, which poll slowly while those reach the page. */
const watched = ["containers", "images", "volumes", "system"]

/**
 * Listen to the manager's event stream while signed in: lists are read again when a job changes,
 * and a toast tells when one ends or an agent asks for a person. After a gap the page reads
 * everything again, and what happened meanwhile is told then, as is what the lists find while the
 * stream is down and they poll.
 */
export function useServerEvents() {
  React.useEffect(() => {
    let jobs: Job[] | undefined
    let requests: HumanRequest[] | undefined
    const unsubscribe = queryClient.getQueryCache().subscribe((event) => {
      if (event.type !== "updated" || event.action.type !== "success") return
      const key = event.query.queryKey
      if (key.length !== 1) return
      if (key[0] === "jobs") {
        const read = event.query.state.data as Job[] | undefined
        if (jobs && read) endedJobs(jobs, read).forEach(announce)
        jobs = read
      } else if (key[0] === HUMAN_REQUESTS[0]) {
        const read = event.query.state.data as HumanRequest[] | undefined
        if (requests && read) newRequests(requests, read).forEach(announceRequest)
        requests = read
      }
    })
    const stop = keepStream({
      open: openEventSource,
      onEvent: handle,
      // Events between the first read of the lists and the stream are not lost; after a gap,
      // nothing the page shows can be trusted.
      onReady: (resumed, dockerEvents) => {
        if (dockerEvents !== undefined) setDockerEvents(dockerEvents)
        if (resumed) void queryClient.invalidateQueries()
        else refresh(streamed)
      },
      // Polling follows the stream: a list read now takes the pace that suits it. The lists
      // Docker changes were just read when the stream comes up, and are read again when it goes.
      onLive: (live) => {
        setStreamLive(live)
        refresh(live ? streamed : [...streamed, ...watched])
      },
      check: sessionHolds,
    })
    return () => {
      stop()
      unsubscribe()
      setStreamLive(false)
      setDockerEvents(false)
    }
  }, [])
}
