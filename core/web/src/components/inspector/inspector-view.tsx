import * as React from "react"
import {
  RiLinkUnlinkM,
  RiPlayLine,
  RiPlugLine,
  RiRefreshLine,
  RiSearchLine,
} from "@remixicon/react"
import { cn } from "cn"

import { ConceptIcon } from "@/components/concept"
import { StatusBadge } from "@/components/status-badge"
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Spinner } from "@/components/ui/spinner"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import type { Tone } from "@/lib/format"
import { t } from "@/lib/i18n"
import {
  expandTemplate,
  fieldChoices,
  filterItems,
  ITEM_KINDS,
  itemId,
  itemKey,
  itemName,
  itemTitle,
  stringValues,
  type Item,
  type ItemKind,
  type ResourceItem,
  type Sort,
  type TargetGroup,
  type TemplateItem,
} from "@/lib/inspector"
import { LIMITS, type InspectorClient, type InspectorState, type Status } from "@/lib/mcp-client"
import type { ContainerItem } from "@/lib/types"
import { ItemDetail } from "./item-detail"
import type { RunState } from "./result-view"
import { TrafficLog } from "./traffic-log"

const SORT_KEY = "thoric.inspector.sort"
const SORTS: Sort[] = ["server", "ascending", "descending"]

function storedSort(): Sort {
  try {
    const value = localStorage.getItem(SORT_KEY)
    return SORTS.includes(value as Sort) ? (value as Sort) : "server"
  } catch {
    return "server"
  }
}

const STATUS: Record<Status, Tone> = {
  connecting: "warning",
  initializing: "warning",
  ready: "success",
  closed: "muted",
}

const SECTIONS = {
  tool: "inspector.section.tools",
  prompt: "inspector.section.prompts",
  resource: "inspector.section.resources",
  template: "inspector.section.templates",
} as const

/** The MCP inspector: open a server, browse what it offers and run it. */
export function InspectorView({
  client,
  groups,
  containers,
  picked,
  onPick,
  onConnect,
  onDisconnect,
}: {
  client: InspectorClient | null
  groups: TargetGroup[]
  /** The containers a field that takes one by its Thoric ID offers by name. */
  containers: ContainerItem[] | undefined
  picked: string | undefined
  onPick(key: string): void
  onConnect(): void
  onDisconnect(): void
}) {
  const choice = groups.flatMap((group) => group.choices).find((option) => option.key === picked)
  const status = useStatus(client)
  const open = status !== undefined && status !== "closed"
  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("inspector.connection.title")}</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="flex flex-wrap items-center gap-2">
            <Select value={picked ?? ""} onValueChange={onPick}>
              <SelectTrigger className="w-full sm:w-96" aria-label={t("inspector.target.label")}>
                <SelectValue placeholder={t("inspector.target.placeholder")} />
              </SelectTrigger>
              <SelectContent>
                {groups.map((group) => (
                  <SelectGroup key={group.label}>
                    <SelectLabel className="flex items-center gap-2">
                      {group.concept && (
                        <ConceptIcon concept={group.concept} className="size-3.5" />
                      )}
                      {group.label}
                    </SelectLabel>
                    {group.choices.map((option) => (
                      <SelectItem
                        key={option.key}
                        value={option.key}
                        disabled={!!option.unavailable}
                        className="font-mono text-xs"
                      >
                        {option.target.kind !== "administrator" && <ConceptIcon concept="mcp" />}
                        {option.label}
                        {option.unavailable && (
                          <span className="font-sans text-muted-foreground">
                            {option.unavailable}
                          </span>
                        )}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                ))}
              </SelectContent>
            </Select>
            <Button type="button" onClick={onConnect} disabled={!choice || !!choice.unavailable}>
              {open ? (
                <RiRefreshLine data-icon="inline-start" />
              ) : (
                <RiPlugLine data-icon="inline-start" />
              )}
              {open ? t("inspector.connection.reconnect") : t("inspector.connection.connect")}
            </Button>
            {open && (
              <Button type="button" variant="outline" onClick={onDisconnect}>
                <RiLinkUnlinkM data-icon="inline-start" />
                {t("inspector.connection.disconnect")}
              </Button>
            )}
          </div>
          {client ? (
            <Connected client={client} />
          ) : (
            <p className="text-xs text-muted-foreground">{t("inspector.connection.hint")}</p>
          )}
        </CardContent>
      </Card>
      {client && <Session key={client.id} client={client} containers={containers} />}
    </div>
  )
}

function useInspector(client: InspectorClient): InspectorState {
  return React.useSyncExternalStore(client.subscribe, client.getState, client.getState)
}

const unsubscribed = () => () => {}

/** The status of the inspector, if one was opened. */
function useStatus(client: InspectorClient | null): Status | undefined {
  const status = () => client?.getState().status
  return React.useSyncExternalStore(client?.subscribe ?? unsubscribed, status, status)
}

/** The opened server: its state, who it is and what it said about itself. */
function Connected({ client }: { client: InspectorClient }) {
  const state = useInspector(client)
  const server = state.server
  const where =
    server?.kind === "administrator" || server?.kind === "endpoint"
      ? server.path
      : server?.container_name
        ? `${server.name} · ${server.container_name}`
        : undefined
  const capabilities = Object.keys(state.capabilities).sort()
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <StatusBadge
          state={state.status}
          tone={STATUS[state.status]}
          label={t(`inspector.status.${state.status}`)}
        />
        {where && <code className="font-mono break-all text-muted-foreground">{where}</code>}
        {state.info?.name && (
          <span>
            {t("inspector.connection.server", {
              name: state.info.title || state.info.name,
              version: state.info.version ?? "",
            })}
          </span>
        )}
        {state.protocolVersion && (
          <span className="text-muted-foreground">
            {t("inspector.connection.protocol", { version: state.protocolVersion })}
          </span>
        )}
        {capabilities.map((name) => (
          <Badge key={name} variant="outline" className="font-mono text-muted-foreground">
            {name}
          </Badge>
        ))}
      </div>
      {state.status === "closed" && state.error && (
        <Alert variant="destructive">
          <AlertTitle>{t("inspector.connection.closed")}</AlertTitle>
          <AlertDescription className="break-words whitespace-pre-wrap">
            {state.error}
          </AlertDescription>
        </Alert>
      )}
      {state.instructions && (
        <Accordion type="multiple" className="rounded-xl border px-3">
          <AccordionItem value="instructions">
            <AccordionTrigger className="py-2 text-xs">
              {t("inspector.connection.instructions")}
            </AccordionTrigger>
            <AccordionContent className="pt-0 pb-3 text-xs whitespace-pre-wrap text-muted-foreground">
              {state.instructions}
            </AccordionContent>
          </AccordionItem>
        </Accordion>
      )}
    </div>
  )
}

type Selected = { kind: ItemKind; key: string; item: Item }

/** What the server offers, the chosen item and the traffic, for one connection. */
function Session({
  client,
  containers,
}: {
  client: InspectorClient
  containers: ContainerItem[] | undefined
}) {
  const state = useInspector(client)
  const choices = React.useMemo(
    () => fieldChoices(client.target, containers ?? []),
    [client.target, containers]
  )
  const [query, setQuery] = React.useState("")
  const [sort, setSort] = React.useState<Sort>(storedSort)
  const [selected, setSelected] = React.useState<Selected>()
  const [runs, setRuns] = React.useState<Record<string, RunState>>({})
  const [args, setArgs] = React.useState<Record<string, Record<string, unknown>>>({})
  const detail = React.useRef<HTMLDivElement>(null)
  const ready = state.status === "ready"

  const changeSort = (value: Sort) => {
    setSort(value)
    try {
      localStorage.setItem(SORT_KEY, value)
    } catch {
      // The order is a convenience; without storage it lasts until the page reloads.
    }
  }

  const choose = (kind: ItemKind, item: Item) => {
    setSelected({ kind, key: itemKey(kind, item), item })
    // Stacked on a narrow page, the detail is below the list.
    if (window.matchMedia("(max-width: 1023px)").matches)
      requestAnimationFrame(() =>
        detail.current?.scrollIntoView({ behavior: "smooth", block: "start" })
      )
  }

  const run = async (kind: ItemKind, item: Item, values: Record<string, unknown>) => {
    const id = itemId(kind, itemKey(kind, item))
    if (runs[id]?.status === "running") return
    const controller = new AbortController()
    const started = Date.now()
    setArgs((current) => ({ ...current, [id]: values }))
    setRuns((current) => ({ ...current, [id]: { status: "running", started, controller } }))
    const options = {
      signal: controller.signal,
      timeout: LIMITS.call,
      onProgress: (progress: { progress: number; total?: number; message?: string }) =>
        setRuns((current) => {
          const now = current[id]
          return now?.status === "running" ? { ...current, [id]: { ...now, progress } } : current
        }),
    }
    let uri: string | undefined
    try {
      let result: unknown
      if (kind === "tool")
        result = await client.request(
          "tools/call",
          { name: itemKey(kind, item), arguments: values },
          options
        )
      else if (kind === "prompt")
        result = await client.request(
          "prompts/get",
          { name: itemKey(kind, item), arguments: stringValues(values) },
          options
        )
      else {
        uri =
          kind === "resource"
            ? (item as ResourceItem).uri
            : expandTemplate((item as TemplateItem).uriTemplate, stringValues(values))
        result = await client.request("resources/read", { uri }, options)
      }
      setRuns((current) => ({
        ...current,
        [id]: {
          status: "done",
          ok: true,
          result,
          durationMs: Date.now() - started,
          at: Date.now(),
          uri,
        },
      }))
    } catch (error) {
      setRuns((current) => ({
        ...current,
        [id]: {
          status: "done",
          ok: false,
          error: (error as Error).message,
          durationMs: Date.now() - started,
          at: Date.now(),
          uri,
        },
      }))
    }
  }

  const cancel = (id: string) => {
    const now = runs[id]
    if (now?.status === "running") now.controller.abort()
  }

  const current = selected && {
    ...selected,
    // The list's entry is the latest: the server may have changed it since it was chosen.
    fresh: (state.lists[selected.kind] as Item[]).find(
      (item) => itemKey(selected.kind, item) === selected.key
    ),
  }
  const currentId = current && itemId(current.kind, current.key)

  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,22rem)_minmax(0,1fr)]">
      <Card className="min-w-0 lg:max-h-[48rem]">
        <CardHeader className="gap-3">
          <div className="flex items-center justify-between gap-2">
            <CardTitle>{t("inspector.items.title")}</CardTitle>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  variant="outline"
                  size="icon-sm"
                  disabled={!ready}
                  onClick={() => void client.refresh()}
                  aria-label={t("inspector.items.refresh")}
                >
                  <RiRefreshLine />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("inspector.items.refresh")}</TooltipContent>
            </Tooltip>
          </div>
          <div className="flex gap-2">
            <div className="relative min-w-0 flex-1">
              <RiSearchLine
                aria-hidden
                className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
              />
              <Input
                type="search"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t("inspector.items.searchShort")}
                aria-label={t("inspector.items.search")}
                className="pl-9"
              />
            </div>
            <Select value={sort} onValueChange={(value) => changeSort(value as Sort)}>
              <SelectTrigger className="w-32 shrink-0" aria-label={t("inspector.items.sort")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {SORTS.map((value) => (
                    <SelectItem key={value} value={value}>
                      {t(`inspector.sort.${value}`)}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>
        </CardHeader>
        <CardContent className="flex min-h-0 flex-col gap-4 overflow-auto">
          {state.status === "connecting" || state.status === "initializing" ? (
            <p className="flex items-center gap-2 text-xs text-muted-foreground">
              <Spinner />
              {t(`inspector.status.${state.status}`)}
            </p>
          ) : (
            <ItemSections
              state={state}
              query={query}
              sort={sort}
              selected={currentId}
              ready={ready}
              runs={runs}
              onChoose={choose}
              onRun={(kind, item) => {
                choose(kind, item)
                void run(kind, item, args[itemId(kind, itemKey(kind, item))] ?? {})
              }}
            />
          )}
        </CardContent>
      </Card>
      <Card ref={detail} className="min-w-0 scroll-mt-4">
        <CardContent>
          {current ? (
            <ItemDetail
              key={currentId}
              kind={current.kind}
              item={current.fresh ?? current.item}
              gone={ready && !current.fresh}
              run={runs[currentId!]}
              lastArgs={args[currentId!]}
              ready={ready}
              choices={choices}
              onRun={(values) => void run(current.kind, current.fresh ?? current.item, values)}
              onCancel={() => cancel(currentId!)}
            />
          ) : (
            <Empty>
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <RiPlayLine />
                </EmptyMedia>
                <EmptyTitle>{t("inspector.items.chooseTitle")}</EmptyTitle>
                <EmptyDescription>{t("inspector.items.choose")}</EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </CardContent>
      </Card>
      <Card className="min-w-0 lg:col-span-2">
        <CardHeader>
          <CardTitle>{t("inspector.log.title")}</CardTitle>
        </CardHeader>
        <CardContent>
          <TrafficLog client={client} log={state.log} logging={!!state.capabilities.logging} />
        </CardContent>
      </Card>
    </div>
  )
}

function ItemSections({
  state,
  query,
  sort,
  selected,
  ready,
  runs,
  onChoose,
  onRun,
}: {
  state: InspectorState
  query: string
  sort: Sort
  selected?: string
  ready: boolean
  runs: Record<string, RunState>
  onChoose(kind: ItemKind, item: Item): void
  onRun(kind: ItemKind, item: Item): void
}) {
  const sections = ITEM_KINDS.map((kind) => {
    const all = state.lists[kind] as Item[]
    return { kind, all, shown: filterItems(kind, all, query, sort) }
  }).filter(({ kind, all }) => all.length || state.listErrors[kind] || state.loading[kind])
  if (!sections.length)
    return (
      <p className="text-xs text-muted-foreground">
        {state.status === "closed" ? t("inspector.items.closed") : t("inspector.items.none")}
      </p>
    )
  return sections.map(({ kind, all, shown }) => (
    <section key={kind} className="flex flex-col gap-1.5">
      <h3 className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
        {t(SECTIONS[kind])}
        <span className="font-normal">
          {query.trim()
            ? t("inspector.items.matches", { shown: shown.length, count: all.length })
            : all.length}
        </span>
        {state.loading[kind] && <Spinner className="size-3" />}
      </h3>
      {state.listErrors[kind] && (
        <p className="text-xs break-words text-destructive">{state.listErrors[kind]}</p>
      )}
      <ul className="flex flex-col gap-0.5">
        {shown.map((item) => {
          const key = itemKey(kind, item)
          const id = itemId(kind, key)
          const title = itemTitle(kind, item)
          const running = runs[id]?.status === "running"
          return (
            <li key={id} className="group/item flex items-start gap-1">
              <button
                type="button"
                aria-current={selected === id || undefined}
                onClick={() => onChoose(kind, item)}
                className={cn(
                  "flex min-w-0 flex-1 flex-col gap-0.5 rounded-lg px-2 py-1.5 text-left hover:bg-muted/60 focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none",
                  selected === id && "bg-muted"
                )}
              >
                <span className="font-mono text-xs break-all">{itemName(kind, item)}</span>
                {(title || item.description) && (
                  <span className="line-clamp-2 text-xs text-muted-foreground">
                    {title || item.description}
                  </span>
                )}
              </button>
              {kind !== "template" && (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-sm"
                      className="mt-0.5 shrink-0"
                      disabled={!ready || running}
                      onClick={() => onRun(kind, item)}
                      aria-label={t("inspector.items.runOf", { name: itemName(kind, item) })}
                    >
                      {running ? <Spinner /> : <RiPlayLine />}
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>{t("inspector.items.runHint")}</TooltipContent>
                </Tooltip>
              )}
            </li>
          )
        })}
      </ul>
      {!shown.length && all.length > 0 && (
        <p className="px-2 text-xs text-muted-foreground">{t("inspector.items.noMatch")}</p>
      )}
    </section>
  ))
}
