import * as React from "react"
import { RiAddLine, RiAlertLine } from "@remixicon/react"
import { toast } from "sonner"

import { AppSidebar, pageIcon, pageParent, pageSection, pageTitle } from "@/components/app-sidebar"
import { DialogHost } from "@/components/dialogs/dialog-host"
import { ErrorBoundary } from "@/components/error-boundary"
import { HumanRequestsBanner } from "@/components/human-requests"
import { SiteHeader } from "@/components/site-header"
import { Button } from "@/components/ui/button"
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { Spinner } from "@/components/ui/spinner"
import { closeDialog, openDialog, whenLeaving } from "@/lib/dialogs"
import { t, useLocale } from "@/lib/i18n"
import { closeInspector } from "@/lib/inspector-session"
import { useSession, useSettings } from "@/lib/queries"
import {
  href,
  keepsHeld,
  pageKey,
  parseRoute,
  replaceRoute,
  ShownRoute,
  useAddress,
  type Page,
} from "@/lib/route"
import { useServerEvents } from "@/lib/server-events"
import { ActivityPage } from "@/pages/activity"
import { DashboardPage } from "@/pages/dashboard"
import { DependenciesPage } from "@/pages/dependencies"
import { FilesPage } from "@/pages/files"
import { ContainersPage } from "@/pages/containers"
import { GroupsPage, NewGroupButton } from "@/pages/groups"
import { ImagesPage } from "@/pages/images"
import { JobsPage } from "@/pages/jobs"
import { LoginPage } from "@/pages/login"
import { McpEndpointsPage } from "@/pages/mcp-endpoints"
import { McpInspectorPage } from "@/pages/mcp-inspector"
import { McpServersPage } from "@/pages/mcp-servers"
import { SettingsPage } from "@/pages/settings"
import { TemplatesPage } from "@/pages/templates"
import { VolumesPage } from "@/pages/volumes"

// noVNC is most of the bundle and only the desktop view needs it.
const DesktopPage = React.lazy(() =>
  import("@/pages/desktop").then((module) => ({ default: module.DesktopPage }))
)

const pages: Record<Page, () => React.ReactNode> = {
  dashboard: DashboardPage,
  containers: ContainersPage,
  groups: GroupsPage,
  images: ImagesPage,
  templates: TemplatesPage,
  volumes: VolumesPage,
  "mcp-endpoints": McpEndpointsPage,
  "mcp-servers": McpServersPage,
  "mcp-inspector": McpInspectorPage,
  files: FilesPage,
  jobs: JobsPage,
  activity: ActivityPage,
  dependencies: DependenciesPage,
  settings: SettingsPage,
}

function Loading() {
  return (
    <div className="flex min-h-svh items-center justify-center bg-sidebar">
      <Spinner className="size-6 text-muted-foreground" />
    </div>
  )
}

function ConnectionFailed({ message }: { message: string }) {
  return (
    <Empty className="min-h-svh bg-sidebar">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <RiAlertLine />
        </EmptyMedia>
        <EmptyTitle>{t("app.connectionFailed")}</EmptyTitle>
        <EmptyDescription>{message}</EmptyDescription>
      </EmptyHeader>
    </Empty>
  )
}

function readSidebarCookie() {
  return !document.cookie.split("; ").includes("sidebar_state=false")
}

function Shell({ page }: { page: Page }) {
  const Content = pages[page]
  const section = pageSection(page)
  // The inspector's server stays open while the pages under MCP show, and ends when they go.
  React.useEffect(() => (section === "mcp" ? closeInspector : undefined), [section])
  return (
    <SidebarProvider
      defaultOpen={readSidebarCookie()}
      style={
        {
          "--sidebar-width": "calc(var(--spacing) * 72)",
          "--header-height": "calc(var(--spacing) * 12)",
        } as React.CSSProperties
      }
    >
      <AppSidebar variant="inset" page={page} />
      <SidebarInset>
        <SiteHeader title={pageTitle(page)} icon={pageIcon(page)} parent={pageParent(page)}>
          {page === "containers" && (
            <Button size="sm" onClick={() => openDialog({ kind: "create" })}>
              <RiAddLine data-icon="inline-start" />
              <span className="hidden sm:inline">{t("containers.new")}</span>
              <span className="sr-only sm:hidden">{t("containers.new")}</span>
            </Button>
          )}
          {page === "groups" && <NewGroupButton />}
        </SiteHeader>
        <HumanRequestsBanner />
        {/* A page that fails to render leaves the sidebar, so another page can still open. */}
        <ErrorBoundary key={page} variant="section" className="px-4 py-4 md:py-6 lg:px-6">
          <Content />
        </ErrorBoundary>
      </SidebarInset>
    </SidebarProvider>
  )
}

/**
 * The route the page shows: the address, once the page it leaves lets it go. A page with uploads
 * still going, or a dialog with changes, asks first, and the address goes back when the user stays.
 */
function useShownRoute() {
  const target = href(useAddress())
  const [shown, setShown] = React.useState(() => parseRoute(target))
  React.useEffect(() => {
    if (target === href(shown)) return
    const next = parseRoute(target)
    const within = pageKey(next) === pageKey(shown)
    whenLeaving(
      () => {
        // A dialog belongs to the page it was opened on; a filter or an item opened there keeps it.
        if (!within) closeDialog()
        setShown(next)
      },
      () => replaceRoute(shown),
      within,
      keepsHeld(shown, next)
    )
  }, [target, shown])
  return shown
}

function Authenticated() {
  const route = useShownRoute()
  const settings = useSettings()
  const locale = useLocale()
  useServerEvents()

  React.useEffect(() => {
    document.title = `${route.page === "desktop" ? t("nav.desktop") : pageTitle(route.page)} · Thoric`
  }, [route.page, locale])

  if (settings.isPending) return <Loading />
  if (settings.error) return <ConnectionFailed message={settings.error.message} />
  return (
    <ShownRoute value={route}>
      {route.page === "desktop" ? (
        <ErrorBoundary key={route.id}>
          <React.Suspense fallback={<Loading />}>
            <DesktopPage id={route.id} />
          </React.Suspense>
        </ErrorBoundary>
      ) : (
        <Shell page={route.page} />
      )}
      <DialogHost />
    </ShownRoute>
  )
}

/** Files dropped anywhere else would replace the page with the file; explain where they go. */
function useDropGuard() {
  React.useEffect(() => {
    const files = (event: DragEvent) => event.dataTransfer?.types.includes("Files")
    const over = (event: DragEvent) => {
      if (!files(event)) return
      event.preventDefault()
      event.dataTransfer!.dropEffect = "none"
    }
    const drop = (event: DragEvent) => {
      if (!files(event)) return
      event.preventDefault()
      toast.error(t("uploads.dropHere"))
    }
    document.addEventListener("dragover", over)
    document.addEventListener("drop", drop)
    return () => {
      document.removeEventListener("dragover", over)
      document.removeEventListener("drop", drop)
    }
  }, [])
}

export function App() {
  const session = useSession()
  useDropGuard()
  if (session.isPending) return <Loading />
  if (session.error) return <ConnectionFailed message={session.error.message} />
  return session.data.authenticated ? <Authenticated /> : <LoginPage />
}
