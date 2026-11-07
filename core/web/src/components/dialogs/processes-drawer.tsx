import * as React from "react"
import {
  RiArrowDownSLine,
  RiArrowRightSLine,
  RiArrowUpSLine,
  RiCloseCircleLine,
  RiMoreLine,
  RiPauseLine,
  RiPlayLine,
  RiSearchLine,
  RiShieldUserLine,
  RiSkullLine,
} from "@remixicon/react"
import { useQuery } from "@tanstack/react-query"
import { cn } from "cn"
import { toast } from "sonner"

import { ConceptIcon } from "@/components/concept"
import { Badge } from "@/components/ui/badge"
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
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { api } from "@/lib/api"
import { formatNumber, plural, rich, t, type MessageKey } from "@/lib/i18n"
import {
  needsRoot,
  nextSort,
  processesPath,
  processRows,
  rowOrder,
  signallable,
  signalProcess,
  type ProcessInfo,
  type ProcessList,
  type ProcessRow,
  type SignalName,
  type Sort,
  type SortKey,
} from "@/lib/processes"
import { queryClient, useContainers } from "@/lib/queries"
import { fileSize } from "@/lib/uploads"
import { SideDrawer, type DialogProps } from "./side-drawer"

/** How often the list is read again while it is open and not paused. */
const REFRESH = 2000

const percent = (value: number) =>
  formatNumber(value / 100, { style: "percent", maximumFractionDigits: value < 10 ? 1 : 0 })

/** States worth naming beside a process; running and sleeping ones are the usual. */
const STATES: Record<string, MessageKey> = {
  Z: "processes.state.Z",
  T: "processes.state.T",
  t: "processes.state.T",
  D: "processes.state.D",
}

type Signal = { process: ProcessInfo; signal: SignalName; tree: boolean }

function RoleBadge({ process }: { process: ProcessInfo }) {
  if (process.role === "program") return null
  const desktop = process.role === "desktop"
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Badge variant={desktop ? "secondary" : "outline"} className="h-4 px-1.5 text-[10px]">
          {t(desktop ? "processes.role.desktop" : "processes.role.thoric")}
        </Badge>
      </TooltipTrigger>
      <TooltipContent>
        {t(desktop ? "processes.role.desktopHint" : "processes.role.thoricHint")}
      </TooltipContent>
    </Tooltip>
  )
}

function SortHead({
  sortKey,
  sort,
  onSort,
  className,
  hint,
  children,
}: {
  sortKey: SortKey
  sort: Sort
  onSort: (key: SortKey) => void
  className?: string
  hint?: string
  children: React.ReactNode
}) {
  const active = sort.key === sortKey
  const Arrow = sort.descending ? RiArrowDownSLine : RiArrowUpSLine
  return (
    <TableHead
      className={className}
      title={hint}
      aria-sort={active ? (sort.descending ? "descending" : "ascending") : undefined}
    >
      <button
        type="button"
        onClick={() => onSort(sortKey)}
        className={cn(
          "inline-flex items-center gap-0.5 rounded-sm outline-none hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50",
          active && "text-foreground"
        )}
      >
        {children}
        <Arrow aria-hidden className={cn("size-3.5", !active && "invisible")} />
      </button>
    </TableHead>
  )
}

function ProcessMenu({
  process,
  childCount,
  onSignal,
  onOpenChange,
}: {
  process: ProcessInfo
  childCount: number
  onSignal: (signal: Signal) => void
  onOpenChange: (open: boolean) => void
}) {
  if (!signallable(process))
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <span tabIndex={0} className="inline-flex size-7 items-center justify-center rounded-md">
            <RiShieldUserLine aria-hidden className="size-4 text-muted-foreground" />
            <span className="sr-only">{t("processes.first")}</span>
          </span>
        </TooltipTrigger>
        <TooltipContent>{t("processes.first")}</TooltipContent>
      </Tooltip>
    )
  const send = (signal: SignalName, tree: boolean) => () => onSignal({ process, signal, tree })
  return (
    <DropdownMenu onOpenChange={onOpenChange}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("containers.actionsOf", { name: `${process.name} (${process.pid})` })}
        >
          <RiMoreLine />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-auto max-w-72">
        {needsRoot(process) && (
          <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
            {t("processes.asRoot", { user: process.user })}
          </DropdownMenuLabel>
        )}
        <DropdownMenuItem tone="warning" onSelect={send("term", false)}>
          <RiCloseCircleLine />
          {t("processes.end")}
        </DropdownMenuItem>
        <DropdownMenuItem variant="destructive" onSelect={send("kill", false)}>
          <RiSkullLine />
          {t("processes.kill")}
        </DropdownMenuItem>
        {childCount > 0 && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem tone="warning" onSelect={send("term", true)}>
              <RiCloseCircleLine />
              {t("processes.endTree")}
            </DropdownMenuItem>
            <DropdownMenuItem variant="destructive" onSelect={send("kill", true)}>
              <RiSkullLine />
              {t("processes.killTree")}
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function ProcessTableRow({
  row,
  tree,
  folded,
  onFold,
  childCount,
  onSignal,
  onMenu,
}: {
  row: ProcessRow
  tree: boolean
  folded: boolean
  onFold: () => void
  childCount: number
  onSignal: (signal: Signal) => void
  onMenu: (open: boolean) => void
}) {
  const { process, depth, children, totals } = row
  // A folded process shows what it uses together with what is below it.
  const summed = tree && folded && children > 0
  const cpu = summed ? totals.cpu : process.cpu_percent
  const rss = summed ? totals.rss : process.rss_bytes
  const state = STATES[process.state]
  return (
    <TableRow>
      <TableCell className="max-w-0 py-1.5">
        <div className="flex min-w-0 items-start gap-1" style={{ paddingLeft: depth * 14 }}>
          {tree &&
            (children > 0 ? (
              <button
                type="button"
                onClick={onFold}
                aria-expanded={!folded}
                aria-label={t(folded ? "processes.unfold" : "processes.fold", {
                  name: process.name,
                })}
                className="mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50"
              >
                {folded ? <RiArrowRightSLine /> : <RiArrowDownSLine />}
              </button>
            ) : (
              <span aria-hidden className="size-4 shrink-0" />
            ))}
          <div className="flex min-w-0 flex-col">
            <span className="flex min-w-0 items-center gap-1.5">
              <span className="truncate font-medium">{process.name}</span>
              <RoleBadge process={process} />
              {state && (
                <Badge variant="outline" className="h-4 px-1.5 text-[10px]">
                  {t(state)}
                </Badge>
              )}
              {summed && (
                <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                  {t("processes.withChildren", { count: formatNumber(totals.count - 1) })}
                </span>
              )}
            </span>
            <span
              className="truncate font-mono text-[11px] text-muted-foreground"
              title={process.command}
            >
              <span className="sm:hidden">{process.pid} · </span>
              {process.command}
            </span>
          </div>
        </div>
      </TableCell>
      <TableCell className="hidden w-24 truncate text-muted-foreground md:table-cell">
        {process.user}
      </TableCell>
      <TableCell className="w-14 px-2 text-right tabular-nums sm:w-16 sm:px-3">
        {percent(cpu)}
      </TableCell>
      <TableCell className="w-20 px-2 text-right tabular-nums sm:w-24 sm:px-3">
        {fileSize(rss)}
      </TableCell>
      <TableCell className="hidden w-16 text-right text-muted-foreground tabular-nums sm:table-cell">
        {process.pid}
      </TableCell>
      <TableCell className="w-10 px-1 py-1 text-right sm:w-12 sm:px-2">
        <ProcessMenu
          process={process}
          childCount={childCount}
          onSignal={onSignal}
          onOpenChange={onMenu}
        />
      </TableCell>
    </TableRow>
  )
}

/** Every process of a running container, refreshed while open, with what each uses. */
export function ProcessesDrawer({ id, ...props }: DialogProps & { id: string }) {
  const container = useContainers(false).data?.find((item) => item.id === id)
  const [paused, setPaused] = React.useState(false)
  const list = useQuery({
    queryKey: ["processes", id],
    queryFn: () => api<ProcessList>(processesPath(id)),
    refetchInterval: props.open && !paused ? REFRESH : false,
    staleTime: 0,
    meta: { quiet: true },
  })
  const [search, setSearch] = React.useState("")
  const [tree, setTree] = React.useState(false)
  const [sort, setSort] = React.useState<Sort>({ key: "cpu", descending: true })
  const [folded, setFolded] = React.useState<ReadonlySet<number>>(new Set())
  const [confirm, setConfirm] = React.useState<Signal | null>(null)
  const [busy, setBusy] = React.useState(false)

  // While the pointer or the keyboard is in the list, or a process's menu is open, the rows keep
  // their places as their figures change, so the one aimed at does not move away.
  const [held, setHeld] = React.useState<ReadonlyMap<number, number> | null>(null)
  const holding = React.useRef({ pointer: false, focus: false, menus: 0 })
  const data = list.data
  const rows = React.useMemo(
    () => (data ? processRows(data.processes, { sort, query: search, tree, folded, held }) : []),
    [data, sort, search, tree, folded, held]
  )
  const shownRows = React.useRef(rows)
  React.useEffect(() => {
    shownRows.current = rows
  }, [rows])
  const hold = () => setHeld((order) => order ?? rowOrder(shownRows.current))
  const release = () => {
    const { pointer, focus, menus } = holding.current
    if (!pointer && !focus && !menus) setHeld(null)
  }
  const onMenu = (open: boolean) => {
    holding.current.menus = Math.max(0, holding.current.menus + (open ? 1 : -1))
    if (open) hold()
    else release()
  }
  // A new order asked for replaces the one held.
  const reorder = (change: () => void) => {
    setHeld(null)
    change()
  }
  const sortBy = (key: SortKey) => reorder(() => setSort(nextSort(sort, key)))
  // A process's children whichever view shows it, for its menu's "with its children".
  const childCounts = React.useMemo(() => {
    const counts = new Map<number, number>()
    for (const process of data?.processes ?? [])
      counts.set(process.ppid, (counts.get(process.ppid) ?? 0) + 1)
    return counts
  }, [data])

  const send = async ({ process, signal, tree: withChildren }: Signal) => {
    setBusy(true)
    try {
      const result = await signalProcess(id, process, signal, withChildren)
      const name = process.name
      const refused = result.refused.length
        ? plural(result.refused.length, {
            one: "processes.refused.one",
            other: "processes.refused.other",
          })
        : undefined
      if (!result.running.length)
        toast.success(t("processes.ended", { name }), { description: refused })
      else
        toast.warning(t("processes.stillRunning", { name }), {
          description: refused,
          action:
            signal === "kill"
              ? undefined
              : {
                  label: t("processes.kill"),
                  onClick: () => void send({ process, signal: "kill", tree: withChildren }),
                },
        })
    } catch (error) {
      toast.error((error as Error).message)
    } finally {
      setBusy(false)
      void queryClient.invalidateQueries({ queryKey: ["processes", id] })
    }
  }
  const signal = (request: Signal) => {
    if (request.process.role === "desktop") setConfirm(request)
    else void send(request)
  }

  const memoryUsed = data ? (data.memory_bytes ?? data.rss_bytes) : 0
  const shown = data ? data.processes.length : 0

  return (
    <SideDrawer {...props} wide className="data-[vaul-drawer-direction=right]:sm:max-w-3xl">
      <DrawerHeader className="gap-1">
        <DrawerTitle>{t("nav.processes")}</DrawerTitle>
        <DrawerDescription asChild>
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
            <span className="flex min-w-0 items-center gap-1.5">
              <ConceptIcon concept="container" className="size-4 shrink-0" />
              <span className="truncate">{container?.spec.name ?? id}</span>
            </span>
            {data ? (
              <>
                <span>
                  {plural(data.total, {
                    one: "processes.count.one",
                    other: "processes.count.other",
                  })}
                </span>
                <span title={t("processes.cpuHint")}>
                  {t("processes.cpuTotal", {
                    value: percent(data.cpu_percent),
                    cpus: formatNumber(data.cpus),
                  })}
                </span>
                <span>
                  {data.memory_limit_bytes
                    ? t("processes.memoryOf", {
                        value: fileSize(memoryUsed),
                        limit: fileSize(data.memory_limit_bytes),
                      })
                    : t("processes.memoryTotal", { value: fileSize(memoryUsed) })}
                </span>
              </>
            ) : (
              list.isPending && <Skeleton className="h-4 w-48" />
            )}
          </div>
        </DrawerDescription>
      </DrawerHeader>
      <div className="flex flex-wrap items-center gap-2 px-4 pb-2">
        <div className="relative min-w-40 flex-1">
          <RiSearchLine
            aria-hidden
            className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder={t("processes.search")}
            aria-label={t("processes.search")}
            className="h-8 pl-8"
          />
        </div>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          value={tree ? "tree" : "list"}
          onValueChange={(value) => value && reorder(() => setTree(value === "tree"))}
          aria-label={t("processes.view")}
        >
          <ToggleGroupItem value="list">{t("processes.view.list")}</ToggleGroupItem>
          <ToggleGroupItem value="tree">{t("processes.view.tree")}</ToggleGroupItem>
        </ToggleGroup>
        <Button
          variant="outline"
          size="sm"
          aria-pressed={paused}
          onClick={() => setPaused(!paused)}
        >
          {paused ? (
            <RiPlayLine data-icon="inline-start" />
          ) : (
            <RiPauseLine data-icon="inline-start" />
          )}
          {t(paused ? "processes.resume" : "processes.pause")}
        </Button>
      </div>
      {confirm && (
        <div className="mx-4 mb-2 flex flex-wrap items-center justify-between gap-2 rounded-lg border border-destructive/40 p-3 text-sm">
          <span>
            {rich("processes.confirm", {
              name: <strong className="font-medium">{confirm.process.name}</strong>,
            })}
          </span>
          <div className="flex gap-2">
            <Button
              variant="destructive"
              size="sm"
              disabled={busy}
              onClick={() => {
                const request = confirm
                setConfirm(null)
                void send(request)
              }}
            >
              {t(confirm.signal === "kill" ? "processes.kill" : "processes.end")}
            </Button>
            <Button variant="outline" size="sm" onClick={() => setConfirm(null)}>
              {t("common.cancel")}
            </Button>
          </div>
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto px-4 text-sm">
        {list.error ? (
          <p className="rounded-lg border p-3 text-muted-foreground">{list.error.message}</p>
        ) : !data ? (
          <div className="flex flex-col gap-2">
            {Array.from({ length: 8 }, (_, index) => (
              <Skeleton key={index} className="h-9 w-full" />
            ))}
          </div>
        ) : (
          <Table className="table-fixed">
            <TableHeader className="sticky top-0 z-10 bg-background">
              <TableRow>
                <SortHead sortKey="name" sort={sort} onSort={sortBy}>
                  {t("processes.column.name")}
                </SortHead>
                <SortHead
                  sortKey="user"
                  sort={sort}
                  onSort={sortBy}
                  className="hidden w-24 md:table-cell"
                >
                  {t("processes.column.user")}
                </SortHead>
                <SortHead
                  sortKey="cpu"
                  sort={sort}
                  onSort={sortBy}
                  className="w-14 px-2 text-right sm:w-16 sm:px-3"
                  hint={t("processes.cpuHint")}
                >
                  {t("processes.column.cpu")}
                </SortHead>
                <SortHead
                  sortKey="memory"
                  sort={sort}
                  onSort={sortBy}
                  className="w-20 px-2 text-right sm:w-24 sm:px-3"
                  hint={t("processes.memoryHint")}
                >
                  {t("processes.column.memory")}
                </SortHead>
                <SortHead
                  sortKey="pid"
                  sort={sort}
                  onSort={sortBy}
                  className="hidden w-16 text-right sm:table-cell"
                >
                  {t("processes.column.pid")}
                </SortHead>
                <TableHead className="w-10 px-1 sm:w-12 sm:px-2">
                  <span className="sr-only">{t("processes.column.actions")}</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody
              onPointerEnter={() => {
                holding.current.pointer = true
                hold()
              }}
              onPointerLeave={() => {
                holding.current.pointer = false
                release()
              }}
              onFocus={() => {
                holding.current.focus = true
                hold()
              }}
              onBlur={(event) => {
                if (event.currentTarget.contains(event.relatedTarget)) return
                holding.current.focus = false
                release()
              }}
            >
              {rows.length ? (
                rows.map((row) => (
                  <ProcessTableRow
                    key={row.process.pid}
                    row={row}
                    tree={tree}
                    folded={folded.has(row.process.pid)}
                    childCount={childCounts.get(row.process.pid) ?? 0}
                    onFold={() => {
                      const next = new Set(folded)
                      if (!next.delete(row.process.pid)) next.add(row.process.pid)
                      setFolded(next)
                    }}
                    onSignal={signal}
                    onMenu={onMenu}
                  />
                ))
              ) : (
                <TableRow>
                  <TableCell colSpan={6} className="py-6 text-center text-muted-foreground">
                    {t("processes.none")}
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        )}
      </div>
      <DrawerFooter className="flex-row flex-wrap items-center justify-end">
        {data && data.total > shown && (
          <span className="mr-auto text-xs text-muted-foreground">
            {t("processes.shown", {
              shown: formatNumber(shown),
              total: formatNumber(data.total),
            })}
          </span>
        )}
        <DrawerClose asChild>
          <Button variant="outline">{t("common.close")}</Button>
        </DrawerClose>
      </DrawerFooter>
    </SideDrawer>
  )
}
