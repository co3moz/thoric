import { QueryCache, QueryClient, useQuery } from "@tanstack/react-query"
import { toast } from "sonner"

import { api, onUnauthorized } from "./api"
import type { DependencyList } from "./dependencies"
import { closeDialog } from "./dialogs"
import { dockerEventsLive, streamLive } from "./events"
import { jobActive } from "./format"
import { fetchTemplateList } from "./templates"
import { followLog, jobReadPath, logWentBack, type JobRead } from "./job-log"
import { endpointApi } from "./mcp"
import { creationEnded } from "./pending"
import type { ScaleSet } from "./scale"
import type { Skill } from "./skills"
import type {
  Configuration,
  ContainerItem,
  DesktopServer,
  EndpointTool,
  ImageItem,
  ImageLayers,
  Job,
  McpEndpoint,
  McpScopes,
  Session,
  Settings,
  SystemInfo,
  UpdateStatus,
  Volume,
} from "./types"

const LIST_INTERVAL = 8000
const JOB_INTERVAL = 2000
/** How often a list the event stream keeps current is read anyway while the stream is up. */
const STREAMED_INTERVAL = 60_000

/** The pace of a list the event stream covers: slow while it is up, as before while it is down. */
export const streamedInterval = (poll: boolean) =>
  poll ? (streamLive() ? STREAMED_INTERVAL : LIST_INTERVAL) : false

/** The pace of a list Docker changes by itself: slow while its events reach the page. */
export const watchedInterval = (poll: boolean) =>
  poll ? (dockerEventsLive() ? STREAMED_INTERVAL : LIST_INTERVAL) : false

let signedOut = false
// While an update replaces the manager, requests fail until the page reloads; see lib/update.ts.
let restarting = false

export function quietWhileRestarting(on: boolean) {
  restarting = on
}

export const queryClient = new QueryClient({
  queryCache: new QueryCache({
    // Several lists fail together when Docker is down; one toast per distinct message is enough.
    // A query marked quiet shows its error on its own page instead.
    onError: (error, query) => {
      if (!signedOut && !restarting && !query.meta?.quiet)
        toast.error(error.message, { id: error.message })
    },
  }),
  defaultOptions: {
    queries: { retry: false, staleTime: 1000, refetchOnWindowFocus: true },
  },
})

onUnauthorized(() => {
  // Requests still in flight from the old session must not toast over the sign-in page.
  signedOut = true
  closeDialog()
  queryClient.removeQueries({ predicate: (query) => query.queryKey[0] !== "session" })
  queryClient.setQueryData<Session>(["session"], (session) => ({
    auth_enabled: session?.auth_enabled ?? true,
    authenticated: false,
  }))
})

export function signedIn() {
  signedOut = false
}

export const useSession = () =>
  useQuery({ queryKey: ["session"], queryFn: () => api<Session>("session"), staleTime: Infinity })

export const useSettings = () =>
  useQuery({
    queryKey: ["settings"],
    queryFn: () => api<Settings>("settings"),
    staleTime: Infinity,
  })

/** The installation's settings, which the manager reads once when it starts. */
export const useConfiguration = () =>
  useQuery({
    queryKey: ["configuration"],
    queryFn: () => api<Configuration>("configuration"),
    staleTime: Infinity,
    meta: { quiet: true },
  })

/** The skills this manager serves its agents, which only a new manager changes. */
export const useSkills = () =>
  useQuery({
    queryKey: ["skills"],
    queryFn: () => api<Skill[]>("skills"),
    staleTime: Infinity,
    meta: { quiet: true },
  })

/** What this manager was built from, which only a new manager changes. */
export const useDependencies = () =>
  useQuery({
    queryKey: ["dependencies"],
    queryFn: () => api<DependencyList>("dependencies"),
    staleTime: Infinity,
    meta: { quiet: true },
  })

// Lists refresh while a page that shows them is open and visible; the desktop view opts out.
// Docker's own events say when they change, so they are polled slowly while those arrive.
function useList<T>(path: string, poll: boolean) {
  return useQuery({
    queryKey: [path],
    queryFn: () => api<T>(path),
    refetchInterval: () => watchedInterval(poll),
  })
}

export const useContainers = (poll = true) => useList<ContainerItem[]>("containers", poll)
export const useImages = (poll = true) => useList<ImageItem[]>("images", poll)
export const useVolumes = (poll = true) => useList<Volume[]>("volumes", poll)
export const useSystem = (poll = true) => useList<SystemInfo>("system", poll)
/** The scale sets with their instances, which the containers page groups its cards by. */
export const useScaleSets = (poll = true) =>
  useQuery({
    queryKey: ["scale-sets"],
    queryFn: async () => (await api<{ items: ScaleSet[] }>("scale-sets")).items,
    refetchInterval: () => watchedInterval(poll),
  })

/**
 * The newest release, which the manager looks up every six hours, and a running update, which is
 * followed closely.
 */
export const useUpdate = () =>
  useQuery({
    queryKey: ["update"],
    queryFn: () => api<UpdateStatus>("update"),
    refetchInterval: (query) => (query.state.data?.job_id ? JOB_INTERVAL : 10 * 60_000),
    staleTime: 60_000,
    meta: { quiet: true },
  })

/**
 * The latest jobs. When one that makes a container or an image ends, the lists are read again
 * before the jobs are, so what it made takes the place of its pending entry without a gap. The
 * event stream says when a job changes, so they are polled slowly while it is up.
 */
export const useJobs = (poll = true) =>
  useQuery({
    queryKey: ["jobs"],
    queryFn: async () => {
      const jobs = await api<Job[]>("jobs")
      const before = queryClient.getQueryData<Job[]>(["jobs"])
      if (before && creationEnded(before, jobs))
        await Promise.all(
          ["containers", "images"].map((key) =>
            queryClient.invalidateQueries({ queryKey: [key], exact: true })
          )
        )
      return jobs
    },
    refetchInterval: () => streamedInterval(poll),
  })

/**
 * One job with its log, followed until it finishes while `open`: a closed drawer stays mounted
 * while it animates out, and must not keep reading. After the first read, each one asks only for
 * what the log gained since; see lib/job-log.ts.
 */
export const useJob = (id: string | undefined, open = true) =>
  useQuery({
    queryKey: ["jobs", id],
    queryFn: async () => {
      const before = queryClient.getQueryData<JobRead>(["jobs", id])
      const read = await api<JobRead>(jobReadPath(id!, before))
      if (logWentBack(before, read)) return api<JobRead>(jobReadPath(id!, undefined))
      return followLog(before, read)
    },
    enabled: !!id && open,
    refetchInterval: (query) =>
      query.state.data && !jobActive(query.state.data) ? false : JOB_INTERVAL,
  })

/** What an image's disk space is made of. It changes only when images are added or removed. */
export const useImageLayers = (id: string) =>
  useQuery({
    queryKey: ["images", id, "layers"],
    queryFn: () => api<ImageLayers>(`images/${encodeURIComponent(id)}/layers`),
    meta: { quiet: true },
  })

/**
 * Every container's desktop MCP servers, as the manager last read them. Their state changes inside
 * the desktops, which the event stream does not report, so a page that shows them polls.
 */
export const useMcpServers = (poll = true) =>
  useQuery({
    queryKey: ["mcp", "servers"],
    queryFn: () => api<{ items: DesktopServer[] }>("mcp/servers").then((data) => data.items),
    refetchInterval: poll ? LIST_INTERVAL : false,
  })

/**
 * The MCP endpoints. They change with this page's own actions and with the jobs that create and
 * delete containers, which the event stream reports, so they are polled slowly while it is up.
 */
export const useMcpEndpoints = (poll = true) =>
  useQuery({
    queryKey: ["mcp", "endpoints"],
    queryFn: () => api<{ items: McpEndpoint[] }>(endpointApi()).then((data) => data.items),
    refetchInterval: () => streamedInterval(poll),
  })

/** The tools an endpoint exposes, exactly as an agent there sees them. */
export const useEndpointTools = (name: string) =>
  useQuery({
    queryKey: ["mcp", "endpoints", name, "tools"],
    queryFn: () =>
      api<{ items: EndpointTool[] }>(`${endpointApi(name)}/tools`).then((data) => data.items),
    meta: { quiet: true },
  })

/** The scopes a grant can have; they change only with the manager's release. */
export const useMcpScopes = () =>
  useQuery({
    queryKey: ["mcp", "scopes"],
    queryFn: () => api<McpScopes>("mcp/scopes"),
    staleTime: Infinity,
  })

/** The templates page's list, read in the browser from the host the manager names. */
export const useTemplates = (list: string | null | undefined) =>
  useQuery({
    queryKey: ["templates", list],
    queryFn: () => fetchTemplateList(list!),
    enabled: !!list,
    staleTime: 5 * 60_000,
    meta: { quiet: true },
  })

/**
 * Reads again what an action changed by itself: the jobs once it queued one, or the lists it
 * changed. What a job changes is read when its end is heard on the event stream (`staleKeys`), or
 * by the lists' own polling while the stream is down, so nothing else, such as the template list
 * from another host, is asked for again.
 */
export const refreshLists = (...keys: string[]) =>
  Promise.all(
    keys.map((key) =>
      // The MCP pages keep several lists under one key.
      queryClient.invalidateQueries({ queryKey: [key], exact: key !== "mcp" })
    )
  )
