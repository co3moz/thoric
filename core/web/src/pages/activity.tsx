import * as React from "react"
import {
  RiAlertLine,
  RiDownloadLine,
  RiErrorWarningLine,
  RiFilterOffLine,
  RiPulseLine,
  RiSearchLine,
} from "@remixicon/react"

import { ActivityDetail } from "@/components/activity-detail"
import { ActivityFeed } from "@/components/activity-feed"
import { ConceptIcon } from "@/components/concept"
import { SideDrawer } from "@/components/dialogs/side-drawer"
import { PageBody } from "@/components/page"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  DrawerClose,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { Toggle } from "@/components/ui/toggle"
import {
  ADMINISTRATOR,
  callerFilter,
  exactTime,
  exportPath,
  filterEndpoint,
  filtersActive,
  toolLabel,
  type ActivityEntry,
  type ActivityFilters,
  type ExportFormat,
} from "@/lib/activity"
import { useActivity } from "@/lib/activity-queries"
import { openDialog } from "@/lib/dialogs"
import { formatNumber, t } from "@/lib/i18n"
import { useContainers, useMcpEndpoints } from "@/lib/queries"
import { navigate, replaceRoute, useRoute } from "@/lib/route"

const ALL = "all"

/** Lets the browser save the log as the manager streams it, under the name it gives. */
function save(filters: ActivityFilters, format: ExportFormat) {
  const link = document.createElement("a")
  link.href = `/api/${exportPath(filters, format)}`
  link.download = ""
  document.body.append(link)
  link.click()
  link.remove()
}

/** `value`, once it has stayed the same for `ms`, so typing does not ask for every letter. */
function useSettled<T>(value: T, ms: number) {
  const [settled, setSettled] = React.useState(value)
  React.useEffect(() => {
    const timer = setTimeout(() => setSettled(value), ms)
    return () => clearTimeout(timer)
  }, [value, ms])
  return settled
}

function Filters({
  container,
  caller,
  tool,
  errors,
  destructive,
  onTool,
  onErrors,
  onDestructive,
  onClear,
  shown,
  containerNames,
  containersLoaded,
  seen,
}: {
  container?: string
  caller?: string
  tool: string
  errors: boolean
  destructive: boolean
  onTool: (value: string) => void
  onErrors: (value: boolean) => void
  onDestructive: (value: boolean) => void
  onClear?: () => void
  /** What the list is narrowed to, which a download follows. */
  shown: ActivityFilters
  containerNames: Map<string, string>
  /** Whether the container list was read, so one missing from it is gone. */
  containersLoaded: boolean
  /** Calls already loaded, which name containers no longer listed. */
  seen: ActivityEntry[]
}) {
  // Only their names, which the page's own lists do not change.
  const endpoints = useMcpEndpoints(false).data ?? []
  const containerChoices = [...containerNames].sort((a, b) => a[1].localeCompare(b[1]))
  if (container && !containerNames.has(container)) {
    // A container no longer listed keeps the name its calls recorded.
    const name =
      seen.find((entry) => entry.container_id === container)?.container_name ??
      container.slice(0, 8)
    containerChoices.push([
      container,
      containersLoaded ? t("activity.filter.deletedContainer", { name }) : name,
    ])
  }
  const endpointNames = endpoints.map((endpoint) => endpoint.name).sort()
  const chosenEndpoint = filterEndpoint(caller)
  if (chosenEndpoint && !endpointNames.includes(chosenEndpoint)) endpointNames.push(chosenEndpoint)
  const route = (next: { container?: string; caller?: string }) =>
    replaceRoute({ page: "activity", ...next })
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Select
        value={container ?? ALL}
        onValueChange={(value) => route({ container: value === ALL ? undefined : value, caller })}
      >
        <SelectTrigger
          size="sm"
          className="w-full sm:w-48"
          aria-label={t("activity.filter.container")}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value={ALL}>{t("activity.filter.allContainers")}</SelectItem>
            {containerChoices.map(([id, name]) => (
              <SelectItem key={id} value={id}>
                <ConceptIcon concept="container" />
                {name}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
      <Select
        value={caller ?? ALL}
        onValueChange={(value) => route({ container, caller: value === ALL ? undefined : value })}
      >
        <SelectTrigger
          size="sm"
          className="w-full sm:w-48"
          aria-label={t("activity.filter.caller")}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value={ALL}>{t("activity.filter.allCallers")}</SelectItem>
            <SelectItem value={ADMINISTRATOR}>{t("activity.caller.administrator")}</SelectItem>
          </SelectGroup>
          {endpointNames.length > 0 && (
            <>
              <SelectSeparator />
              <SelectGroup>
                {endpointNames.map((name) => (
                  <SelectItem key={name} value={callerFilter(name)}>
                    <ConceptIcon concept="mcp" />
                    <span className="font-mono">{name}</span>
                  </SelectItem>
                ))}
              </SelectGroup>
            </>
          )}
        </SelectContent>
      </Select>
      <div className="relative w-full sm:w-56">
        <RiSearchLine className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="search"
          aria-label={t("activity.filter.tool")}
          placeholder={t("activity.filter.tool")}
          value={tool}
          onChange={(event) => onTool(event.target.value)}
          className="h-8 pl-8"
        />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Toggle
          variant="outline"
          size="sm"
          pressed={errors}
          onPressedChange={onErrors}
          className="aria-pressed:border-destructive/40 aria-pressed:text-destructive"
        >
          <RiErrorWarningLine data-icon="inline-start" />
          {t("activity.filter.errors")}
        </Toggle>
        <Toggle
          variant="outline"
          size="sm"
          pressed={destructive}
          onPressedChange={onDestructive}
          className="aria-pressed:border-destructive/40 aria-pressed:text-destructive"
        >
          <RiAlertLine data-icon="inline-start" />
          {t("activity.filter.destructive")}
        </Toggle>
        {onClear && (
          <Button variant="ghost" size="sm" onClick={onClear}>
            <RiFilterOffLine data-icon="inline-start" />
            {t("activity.filter.clear")}
          </Button>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" title={t("activity.export.hint")}>
              <RiDownloadLine data-icon="inline-start" />
              {t("activity.export")}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-52">
            <DropdownMenuItem onSelect={() => save(shown, "csv")}>
              {t("activity.export.csv")}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => save(shown, "jsonl")}>
              {t("activity.export.jsonl")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  )
}

function Loading() {
  return (
    <div className="flex flex-col gap-3 px-3" aria-label={t("activity.loading")} role="status">
      {[0, 1, 2, 3, 4].map((row) => (
        <div key={row} className="flex items-start gap-3">
          <Skeleton className="size-7 rounded-full" />
          <div className="flex flex-1 flex-col gap-1.5">
            <Skeleton className="h-4 w-1/3" />
            <Skeleton className="h-3 w-2/3" />
          </div>
          <Skeleton className="h-3 w-14" />
        </div>
      ))}
    </div>
  )
}

/** The agent activity log: every MCP tool call and used transfer link, newest first. */
export function ActivityPage() {
  const route = useRoute()
  const linked = route.page === "activity" ? route : { page: "activity" as const }
  const containers = useContainers().data
  const containerNames = React.useMemo(
    () => new Map((containers ?? []).map((item) => [item.id, item.spec.name])),
    [containers]
  )
  const [tool, setTool] = React.useState("")
  const search = useSettled(tool.trim(), 300)
  const [errors, setErrors] = React.useState(false)
  const [destructive, setDestructive] = React.useState(false)
  const filters: ActivityFilters = {
    container: linked.container,
    caller: linked.caller,
    tool: search,
    errors,
    destructive,
  }
  const activity = useActivity(filters)
  const entries = React.useMemo(
    () => activity.data?.pages.flatMap((page) => page.items) ?? [],
    [activity.data]
  )
  const kept = activity.data?.pages[0]?.kept
  // The address names the call that is open, whether a link or a row opened it, so a link followed
  // while this page shows opens its call too. The last one stays while the panel closes.
  const [picked, setPicked] = React.useState(linked.entry)
  if (linked.entry && linked.entry !== picked) setPicked(linked.entry)
  const selected = entries.find((entry) => entry.id === picked)
  const close = () => replaceRoute({ ...linked, entry: undefined })
  const clear = () => {
    setTool("")
    setErrors(false)
    setDestructive(false)
    replaceRoute({ page: "activity" })
  }
  const active = filtersActive({ ...filters, tool })

  return (
    <PageBody>
      <div className="flex flex-col gap-4 px-4 lg:px-6">
        <p className="text-sm text-muted-foreground">{t("activity.description")}</p>
        <Filters
          container={linked.container}
          caller={linked.caller}
          tool={tool}
          errors={errors}
          destructive={destructive}
          onTool={setTool}
          onErrors={setErrors}
          onDestructive={setDestructive}
          onClear={active ? clear : undefined}
          shown={filters}
          containerNames={containerNames}
          containersLoaded={!!containers}
          seen={entries}
        />
        {activity.isPending ? (
          <Loading />
        ) : activity.error ? (
          <Alert variant="destructive">
            <RiErrorWarningLine />
            <AlertTitle>{t("activity.failed")}</AlertTitle>
            <AlertDescription>{activity.error.message}</AlertDescription>
          </Alert>
        ) : entries.length === 0 ? (
          active ? (
            <Empty className="border">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <RiFilterOffLine />
                </EmptyMedia>
                <EmptyTitle>{t("activity.filtered.title")}</EmptyTitle>
                <EmptyDescription>{t("activity.filtered.description")}</EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <Button variant="outline" onClick={clear}>
                  {t("activity.filter.clear")}
                </Button>
              </EmptyContent>
            </Empty>
          ) : (
            <Empty className="border">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <RiPulseLine />
                </EmptyMedia>
                <EmptyTitle>{t("activity.empty.title")}</EmptyTitle>
                <EmptyDescription>{t("activity.empty.description")}</EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <Button variant="outline" onClick={() => navigate({ page: "mcp-endpoints" })}>
                  <ConceptIcon concept="mcp" data-icon="inline-start" />
                  {t("activity.empty.action")}
                </Button>
              </EmptyContent>
            </Empty>
          )
        ) : (
          <>
            <ActivityFeed
              entries={entries}
              containerNames={containerNames}
              showContainer={!linked.container}
              onSelect={(entry) => replaceRoute({ ...linked, entry: entry.id })}
            />
            <div className="flex flex-col items-center gap-1 pb-2">
              {activity.hasNextPage ? (
                <Button
                  variant="outline"
                  size="sm"
                  disabled={activity.isFetchingNextPage}
                  onClick={() => void activity.fetchNextPage()}
                >
                  {activity.isFetchingNextPage && <Spinner data-icon="inline-start" />}
                  {t("activity.loadOlder")}
                </Button>
              ) : (
                <p className="text-xs text-muted-foreground">
                  {t("activity.end")}
                  {kept !== undefined && ` ${t("activity.kept", { count: formatNumber(kept) })}`}
                </p>
              )}
            </div>
          </>
        )}
      </div>
      <SideDrawer
        open={!!linked.entry && !!selected}
        onOpenChange={(next) => !next && close()}
        wide
      >
        {selected && (
          <>
            <DrawerHeader className="gap-1">
              <DrawerTitle className="font-mono break-all">{toolLabel(selected)}</DrawerTitle>
              <DrawerDescription>{exactTime(selected.at)}</DrawerDescription>
            </DrawerHeader>
            <div className="overflow-y-auto px-4">
              <ActivityDetail
                entry={selected}
                containerName={
                  selected.container_id ? containerNames.get(selected.container_id) : undefined
                }
                containerGone={
                  !!containers &&
                  !!selected.container_id &&
                  !containerNames.has(selected.container_id)
                }
                onContainer={(id) => {
                  close()
                  openDialog({ kind: "details", id })
                }}
                onJob={(id) => {
                  close()
                  openDialog({ kind: "job", id })
                }}
              />
            </div>
            <DrawerFooter>
              <DrawerClose asChild>
                <Button variant="outline">{t("common.done")}</Button>
              </DrawerClose>
            </DrawerFooter>
          </>
        )}
      </SideDrawer>
    </PageBody>
  )
}
