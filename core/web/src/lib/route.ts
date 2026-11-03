import { createContext, useContext, useSyncExternalStore } from "react"

export type Page =
  | "dashboard"
  | "containers"
  | "groups"
  | "images"
  | "templates"
  | "volumes"
  | "mcp-endpoints"
  | "mcp-servers"
  | "mcp-inspector"
  | "files"
  | "jobs"
  | "activity"
  | "dependencies"
  | "settings"
export type Route =
  | { page: Exclude<Page, "files" | "activity" | "groups"> }
  /** The network groups, or the members of the one `group` names. */
  | { page: "groups"; group?: string }
  /**
   * The files page, with the container it shows when one was chosen, and its change list instead
   * of its files with `view`.
   */
  | { page: "files"; container?: string; view?: "changes" }
  /**
   * The agent activity log, narrowed to a container or a caller (`administrator` or
   * `endpoint:NAME`), with one call's details open when `entry` names it.
   */
  | { page: "activity"; container?: string; caller?: string; entry?: number }
  | { page: "desktop"; id: string }

const pages: Page[] = [
  "dashboard",
  "containers",
  "groups",
  "images",
  "templates",
  "volumes",
  "files",
  "jobs",
  "activity",
  "dependencies",
  "settings",
]

/** The MCP pages, each at `#/mcp/` and the part of its name after `mcp-`. */
const mcpPages = ["mcp-endpoints", "mcp-servers", "mcp-inspector"] as const
const isMcpPage = (page: string): page is (typeof mcpPages)[number] =>
  (mcpPages as readonly string[]).includes(page)

/** The activity page's filters, from the part of its link after `?`. */
function activityRoute(search: string): Route {
  const query = new URLSearchParams(search)
  const entry = Number(query.get("entry"))
  return {
    page: "activity",
    ...(query.get("container") ? { container: query.get("container")! } : {}),
    ...(query.get("caller") ? { caller: query.get("caller")! } : {}),
    ...(Number.isSafeInteger(entry) && entry > 0 ? { entry } : {}),
  }
}

/** Pages live in the URL fragment, so the manager serves one index.html for all of them. */
export function parseRoute(hash: string): Route {
  const link = hash.replace(/^#\/?/, "")
  const at = link.indexOf("?")
  const [path, search] = at < 0 ? [link, ""] : [link.slice(0, at), link.slice(at + 1)]
  const [page, id, view] = path.split("/")
  if (page === "desktop" && id) return { page: "desktop", id: decodeURIComponent(id) }
  if (page === "files") {
    if (!id) return { page }
    const container = decodeURIComponent(id)
    return view === "changes" ? { page, container, view } : { page, container }
  }
  if (page === "groups") return id ? { page, group: decodeURIComponent(id) } : { page }
  if (page === "activity") return activityRoute(search)
  if (page === "mcp") {
    // `#/mcp` alone, the MCP page's link before it had pages of its own, opens the endpoints.
    const view = `mcp-${id}`
    return { page: isMcpPage(view) ? view : "mcp-endpoints" }
  }
  return {
    page: pages.includes(page as Page)
      ? (page as Exclude<Page, "files" | "activity" | "groups">)
      : "dashboard",
  }
}

export function href(route: Route) {
  if (route.page === "desktop") return `#/desktop/${encodeURIComponent(route.id)}`
  if (route.page === "files" && route.container)
    return `#/files/${encodeURIComponent(route.container)}${route.view ? `/${route.view}` : ""}`
  if (route.page === "groups" && route.group) return `#/groups/${encodeURIComponent(route.group)}`
  if (route.page === "activity") {
    const query = new URLSearchParams()
    if (route.container) query.set("container", route.container)
    if (route.caller) query.set("caller", route.caller)
    if (route.entry) query.set("entry", String(route.entry))
    const search = query.toString()
    return search ? `#/activity?${search}` : "#/activity"
  }
  if (isMcpPage(route.page)) return `#/mcp/${route.page.slice("mcp-".length)}`
  return `#/${route.page}`
}

/**
 * Whether moving between the two routes leaves everything the page holds where it is: the views
 * of one container on the files page, which both stay open, so uploads there go on.
 */
export const keepsHeld = (from: Route, to: Route) =>
  from.page === "files" &&
  to.page === "files" &&
  !!from.container &&
  from.container === to.container

/** What a dialog belongs to: the page, or the desktop, which a filter or an open item keeps. */
export const pageKey = (route: Route) =>
  route.page === "desktop" ? `desktop/${route.id}` : route.page

export function navigate(route: Route) {
  location.hash = href(route)
}

/** Go to `route` in place of the current entry of the browser's history, as a filter does. */
export function replaceRoute(route: Route) {
  location.replace(href(route))
}

const subscribe = (listener: () => void) => {
  window.addEventListener("hashchange", listener)
  return () => window.removeEventListener("hashchange", listener)
}

/** The route in the address bar. */
export function useAddress(): Route {
  const hash = useSyncExternalStore(subscribe, () => location.hash)
  return parseRoute(hash)
}

/**
 * The route the page shows, which stays while the user is asked whether to leave it, such as with
 * uploads still going; see App.tsx.
 */
export const ShownRoute = createContext<Route | null>(null)

export function useRoute(): Route {
  const address = useAddress()
  return useContext(ShownRoute) ?? address
}
