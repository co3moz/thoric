import * as React from "react"
import { RiAddLine, RiDeleteBinLine, RiPlayLine, RiRefreshLine } from "@remixicon/react"
import { cn } from "cn"
import { toast } from "sonner"

import { ConceptBadge, ConceptIcon, ConceptMedia } from "@/components/concept"
import { DockerAlert, PageBody, SectionHeading } from "@/components/page"
import { StatusBadge } from "@/components/status-badge"
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from "@/components/ui/empty"
import { Spinner } from "@/components/ui/spinner"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { attempt } from "@/lib/actions"
import { openDialog } from "@/lib/dialogs"
import { containerStatus, shortDate } from "@/lib/format"
import { plural, t } from "@/lib/i18n"
import { useOpenInInspector } from "@/lib/inspector-session"
import { serverCommand, serverDetails, serversByContainer, serverTone } from "@/lib/mcp"
import { refreshServer, rereadServers } from "@/lib/mcp-api"
import { useContainers, useMcpServers } from "@/lib/queries"
import type { ContainerItem, DesktopServer } from "@/lib/types"

/** Fetches a server's tools again, and says what came back. */
function RefreshServer({ server }: { server: DesktopServer }) {
  const [busy, setBusy] = React.useState(false)
  const refresh = async () => {
    setBusy(true)
    await attempt(async () => {
      const result = await refreshServer(server.container_id, server.name)
      if (result.state === "error")
        toast.error(t("mcp.server.failed", { name: result.name, error: result.error ?? "" }))
      else
        toast.success(
          plural(
            result.tools.length,
            { one: "mcp.server.read.one", other: "mcp.server.read.other" },
            { name: result.name }
          )
        )
    })
    setBusy(false)
  }
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="outline"
          size="icon-sm"
          disabled={busy || server.state === "stopped" || server.state === "elsewhere"}
          onClick={refresh}
          aria-label={t("mcp.server.refreshOf", { name: server.name })}
        >
          {busy ? <Spinner /> : <RiRefreshLine />}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{t("mcp.server.refresh")}</TooltipContent>
    </Tooltip>
  )
}

/** One desktop MCP server: where it comes from, how it starts, its state, tools and log. */
function ServerRow({ server }: { server: DesktopServer }) {
  const inspect = useOpenInInspector()
  const details = serverDetails(server.env, server.headers, server.cwd)
  // Without a command or a URL its declaration could not be read, so nothing can start it; one
  // that another instance of its set runs is never started here.
  const startable =
    server.state !== "stopped" && server.state !== "elsewhere" && !!(server.command || server.url)
  return (
    <li className="flex flex-col gap-2 px-4 py-3">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <ConceptIcon concept="mcp" className="size-4 shrink-0" />
            <span className="font-mono text-sm font-medium break-all">{server.name}</span>
            {server.source === "image" ? (
              <ConceptBadge concept="image" title={t("mcp.source.imageHint")}>
                {t("mcp.source.image")}
              </ConceptBadge>
            ) : (
              <Badge
                variant="outline"
                className="text-muted-foreground"
                title={t("mcp.source.containerHint")}
              >
                {t("mcp.source.container")}
              </Badge>
            )}
            <Badge variant="outline" className="text-muted-foreground">
              {server.transport === "stdio" ? "stdio" : "HTTP"}
            </Badge>
            <StatusBadge
              state={server.state}
              tone={serverTone(server.state)}
              label={t(`mcp.state.${server.state}`, { number: server.runs_in ?? 1 })}
            />
          </div>
          {server.description && (
            <p className="text-xs text-muted-foreground">{server.description}</p>
          )}
          <code className="font-mono text-[0.7rem] break-all text-muted-foreground">
            {serverCommand({
              command: server.command ?? undefined,
              args: server.args,
              url: server.url ?? undefined,
            })}
          </code>
          {details && <p className="text-xs break-words text-muted-foreground">{details}</p>}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="outline"
                size="icon-sm"
                disabled={!startable}
                onClick={() =>
                  inspect({ kind: "server", container: server.container_id, name: server.name })
                }
                aria-label={t("inspector.openOf", { name: server.name })}
              >
                <RiPlayLine />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{t("inspector.open")}</TooltipContent>
          </Tooltip>
          <RefreshServer server={server} />
          {server.source === "container" && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="icon-sm"
                  className="text-destructive hover:text-destructive"
                  onClick={() =>
                    openDialog({
                      kind: "mcp-server-remove",
                      container: server.container_id,
                      name: server.name,
                    })
                  }
                  aria-label={t("mcp.server.removeOf", { name: server.name })}
                >
                  <RiDeleteBinLine />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("mcp.server.remove")}</TooltipContent>
            </Tooltip>
          )}
        </div>
      </div>
      {server.error && (
        <p className="text-xs break-words whitespace-pre-wrap text-destructive">{server.error}</p>
      )}
      {server.tools.length || server.log ? (
        <Accordion type="multiple" className="rounded-xl border px-3">
          {server.tools.length > 0 && (
            <AccordionItem value="tools">
              <AccordionTrigger className="py-2 text-xs">
                <span className="flex flex-wrap items-center gap-x-2">
                  {plural(server.tools.length, { one: "mcp.tools.one", other: "mcp.tools.other" })}
                  {server.tools_at !== null && (
                    <span className="font-normal text-muted-foreground">
                      {t("mcp.server.readAt", { date: shortDate(server.tools_at) })}
                    </span>
                  )}
                </span>
              </AccordionTrigger>
              <AccordionContent className="pt-0 pb-2">
                <ul className="flex flex-col divide-y">
                  {server.tools.map((tool) => (
                    <li key={tool.name} className="flex flex-col gap-0.5 py-1.5">
                      <code className="font-mono text-xs break-all">{tool.name}</code>
                      {tool.description && (
                        <p className="line-clamp-3 text-xs text-muted-foreground">
                          {tool.description}
                        </p>
                      )}
                    </li>
                  ))}
                </ul>
              </AccordionContent>
            </AccordionItem>
          )}
          {server.log && (
            <AccordionItem value="log">
              <AccordionTrigger className="py-2 text-xs">{t("mcp.server.log")}</AccordionTrigger>
              <AccordionContent className="pt-0 pb-2">
                <pre className="max-h-64 overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-xs leading-relaxed break-words whitespace-pre-wrap">
                  {server.log}
                </pre>
              </AccordionContent>
            </AccordionItem>
          )}
        </Accordion>
      ) : (
        <p className="text-xs text-muted-foreground">
          {server.state === "elsewhere"
            ? t("mcp.server.elsewhere", { number: server.runs_in ?? 1 })
            : server.state === "ready"
              ? t("mcp.server.noTools")
              : t("mcp.server.toolsUnread")}
        </p>
      )}
    </li>
  )
}

/** Reads a container's servers again from its desktop, such as ones added with thoric-mcp. */
function RereadServers({ id, name }: { id: string; name: string }) {
  const [busy, setBusy] = React.useState(false)
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="outline"
          size="icon-sm"
          disabled={busy}
          onClick={async () => {
            setBusy(true)
            await attempt(() => rereadServers(id))
            setBusy(false)
          }}
          aria-label={t("mcp.servers.rereadOf", { name })}
        >
          {busy ? <Spinner /> : <RiRefreshLine />}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{t("mcp.servers.reread")}</TooltipContent>
    </Tooltip>
  )
}

function ServersView({ containers }: { containers: ContainerItem[] }) {
  const servers = useMcpServers()
  // Containers with servers come first; the others keep a short line each, with Add server.
  const groups = serversByContainer(servers.data ?? [], containers).sort(
    (a, b) => Number(!a.servers.length) - Number(!b.servers.length)
  )
  if (servers.isPending)
    return (
      <p className="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground">
        <Spinner />
        {t("mcp.servers.loading")}
      </p>
    )
  if (servers.error)
    return <p className="py-12 text-center text-sm text-destructive">{servers.error.message}</p>
  if (!groups.length)
    return (
      <Empty className="border">
        <EmptyHeader>
          <ConceptMedia concept="container" />
          <EmptyTitle>{t("containers.none")}</EmptyTitle>
          <EmptyDescription>{t("mcp.servers.noContainers")}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  return (
    <div className="flex flex-col gap-4">
      {groups.map((group) => {
        const status = group.container && containerStatus(group.container)
        const running = group.container?.state === "running"
        const name = group.container?.spec.name ?? group.name
        return (
          <section key={group.id} className="flex flex-col rounded-2xl border">
            <header
              className={cn(
                "flex flex-wrap items-center justify-between gap-2 px-4 py-3",
                group.servers.length > 0 && "border-b"
              )}
            >
              <div className="flex min-w-0 flex-wrap items-center gap-2">
                <ConceptIcon concept="container" className="size-4 shrink-0" />
                <h3 className="text-sm font-medium break-all">{name}</h3>
                {status ? (
                  <StatusBadge state={status.key} tone={status.tone} />
                ) : (
                  <Badge variant="outline" className="text-warning">
                    {t("mcp.grant.deleted")}
                  </Badge>
                )}
                {!group.servers.length && (
                  <span className="text-xs text-muted-foreground">{t("mcp.servers.noneHere")}</span>
                )}
              </div>
              {group.container && (
                <div className="flex items-center gap-2">
                  {running && <RereadServers id={group.id} name={name} />}
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={!running}
                    onClick={() => openDialog({ kind: "mcp-server", container: group.id })}
                    aria-label={t("mcp.server.addTo", { name })}
                  >
                    <RiAddLine data-icon="inline-start" />
                    {t("mcp.server.add")}
                  </Button>
                </div>
              )}
            </header>
            {group.servers.length > 0 && (
              <ul className="flex flex-col divide-y">
                {group.servers.map((server) => (
                  <ServerRow key={server.name} server={server} />
                ))}
              </ul>
            )}
          </section>
        )
      })}
    </div>
  )
}

/** The MCP servers that run inside the desktops, grouped by container. */
export function McpServersPage() {
  const containers = useContainers().data
  return (
    <PageBody>
      <DockerAlert />
      <SectionHeading title={t("mcp.servers.title")} description={t("mcp.servers.description")}>
        <Button size="sm" onClick={() => openDialog({ kind: "mcp-server" })}>
          <RiAddLine data-icon="inline-start" />
          {t("mcp.server.add")}
        </Button>
      </SectionHeading>
      <div className="flex flex-col gap-4 px-4 lg:px-6">
        <ServersView containers={containers ?? []} />
      </div>
    </PageBody>
  )
}
