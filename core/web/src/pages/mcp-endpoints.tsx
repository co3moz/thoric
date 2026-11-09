import * as React from "react"
import {
  RiAddLine,
  RiDeleteBinLine,
  RiEditLine,
  RiForbidLine,
  RiInformationLine,
  RiKey2Line,
  RiMore2Line,
  RiPlayLine,
  RiPulseLine,
  RiShareForwardLine,
} from "@remixicon/react"

import { ConceptIcon } from "@/components/concept"
import { CopyButton } from "@/components/copy-button"
import {
  columnHelper,
  DataTablePagination,
  DataTableView,
  SortHeader,
  useDataTable,
  type DataColumn,
} from "@/components/data-table"
import { DockerAlert, PageBody, SectionHeading } from "@/components/page"
import { StatusBadge } from "@/components/status-badge"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { attempt } from "@/lib/actions"
import { openDialog } from "@/lib/dialogs"
import { shortDate } from "@/lib/format"
import { plural, rich, t } from "@/lib/i18n"
import { useOpenInInspector } from "@/lib/inspector-session"
import {
  endpointUrl,
  profileDescription,
  profileLabel,
  scopeSummary,
  serversSummary,
} from "@/lib/mcp"
import { newToken } from "@/lib/mcp-api"
import { useContainers, useMcpEndpoints, useMcpScopes, useSettings } from "@/lib/queries"
import type { McpEndpoint } from "@/lib/types"

type Row = McpEndpoint & {
  url: string
  /** The ids of the containers that still exist. */
  present: Set<string>
}
const column = columnHelper<Row>()

function EndpointMenu({ endpoint }: { endpoint: McpEndpoint }) {
  const inspect = useOpenInInspector()
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          className="flex size-8 text-muted-foreground data-[state=open]:bg-muted"
          size="icon"
        >
          <RiMore2Line />
          <span className="sr-only">{t("mcp.endpoint.actionsOf", { name: endpoint.name })}</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-52">
        <DropdownMenuItem
          tone="success"
          onSelect={() => inspect({ kind: "endpoint", name: endpoint.name })}
        >
          <RiPlayLine />
          {t("inspector.open")}
        </DropdownMenuItem>
        <DropdownMenuItem
          tone="info"
          onSelect={() => openDialog({ kind: "mcp-connect", name: endpoint.name })}
        >
          <RiShareForwardLine />
          {t("mcp.connect.action")}
        </DropdownMenuItem>
        <DropdownMenuItem
          tone="info"
          onSelect={() => openDialog({ kind: "activity", endpoint: endpoint.name })}
        >
          <RiPulseLine />
          {t("activity.endpoint.action")}
        </DropdownMenuItem>
        <DropdownMenuItem
          tone="edit"
          onSelect={() => openDialog({ kind: "mcp-endpoint", name: endpoint.name })}
        >
          <RiEditLine />
          {t("mcp.endpoint.edit")}
        </DropdownMenuItem>
        <DropdownMenuItem tone="success" onSelect={() => attempt(() => newToken(endpoint))}>
          <RiKey2Line />
          {endpoint.has_token ? t("mcp.token.replace") : t("mcp.token.create")}
        </DropdownMenuItem>
        {endpoint.has_token && (
          <DropdownMenuItem
            tone="warning"
            onSelect={() =>
              openDialog({ kind: "mcp-endpoint-confirm", name: endpoint.name, action: "revoke" })
            }
          >
            <RiForbidLine />
            {t("mcp.token.revoke")}
          </DropdownMenuItem>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          variant="destructive"
          onSelect={() =>
            openDialog({ kind: "mcp-endpoint-confirm", name: endpoint.name, action: "delete" })
          }
        >
          <RiDeleteBinLine />
          {t("mcp.endpoint.delete")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

const columns: DataColumn<Row>[] = [
  column.accessor("name", {
    header: ({ column }) => <SortHeader column={column}>{t("table.name")}</SortHeader>,
    cell: ({ row }) => {
      const endpoint = row.original
      return (
        // Table cells do not wrap; the URL does here so the row's actions stay in view.
        <div className="flex min-w-56 flex-col gap-1 whitespace-normal">
          <div className="flex flex-wrap items-center gap-2">
            <ConceptIcon concept="mcp" className="size-4 shrink-0" />
            <span className="font-mono text-xs font-medium">{endpoint.name}</span>
            <Badge variant="outline" className="text-muted-foreground">
              {endpoint.automatic ? t("mcp.endpoint.automatic") : t("mcp.endpoint.custom")}
            </Badge>
          </div>
          <div className="flex items-center gap-1.5">
            <code className="font-mono text-[0.7rem] break-all text-muted-foreground">
              {endpoint.url}
            </code>
            <CopyButton
              size="icon-xs"
              text={endpoint.url}
              label={t("mcp.endpoint.copyUrlOf", { name: endpoint.name })}
            />
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="outline"
                  size="icon-xs"
                  className="shrink-0"
                  onClick={() => openDialog({ kind: "mcp-connect", name: endpoint.name })}
                  aria-label={t("mcp.connect.actionOf", { name: endpoint.name })}
                >
                  <RiShareForwardLine />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("mcp.connect.action")}</TooltipContent>
            </Tooltip>
          </div>
        </div>
      )
    },
  }),
  column.display({
    id: "grants",
    header: () => t("mcp.endpoint.containers"),
    cell: ({ row }) => {
      const { grants, present } = row.original
      if (!grants.length)
        return <span className="text-xs text-muted-foreground">{t("mcp.endpoint.noGrants")}</span>
      return (
        <ul className="flex min-w-56 flex-col gap-1.5 text-xs whitespace-normal">
          {grants.map((grant) => (
            <li key={grant.set ?? grant.container_id} className="flex flex-col gap-0.5">
              <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
                <ConceptIcon concept="container" className="size-3.5 shrink-0" />
                {grant.set ? (
                  <span className="font-medium">{t("scale.setOf", { name: grant.set })}</span>
                ) : present.has(grant.container_id) ? (
                  <button
                    type="button"
                    className="font-medium hover:underline"
                    onClick={() => openDialog({ kind: "details", id: grant.container_id })}
                  >
                    {grant.container_name}
                  </button>
                ) : (
                  <span className="font-medium">{grant.container_name}</span>
                )}
                <code className="font-mono text-muted-foreground">{grant.alias}</code>
                {!grant.set && !present.has(grant.container_id) && (
                  <Badge variant="outline" className="text-warning">
                    {t("mcp.grant.deleted")}
                  </Badge>
                )}
              </span>
              <span className="text-muted-foreground">{scopeSummary(grant.scopes)}</span>
              <span className="text-muted-foreground">
                {t("mcp.grant.serversLine", { servers: serversSummary(grant.servers) })}
              </span>
              {!!grant.profile && grant.profile !== "full" && (
                <span className="text-muted-foreground" title={profileDescription(grant.profile)}>
                  {t("mcp.grant.profileLine", { profile: profileLabel(grant.profile) })}
                </span>
              )}
              {grant.excluded_tools.length > 0 && (
                <span className="text-muted-foreground" title={grant.excluded_tools.join(", ")}>
                  {plural(grant.excluded_tools.length, {
                    one: "mcp.grant.excluded.one",
                    other: "mcp.grant.excluded.other",
                  })}
                </span>
              )}
            </li>
          ))}
        </ul>
      )
    },
  }),
  column.display({
    id: "token",
    header: () => t("mcp.token.column"),
    cell: ({ row }) => {
      const endpoint = row.original
      return endpoint.has_token ? (
        <div className="flex flex-col gap-1">
          <StatusBadge state="token" tone="success" label={t("mcp.token.set")} />
          {endpoint.token_created_at !== null && (
            <span className="text-xs whitespace-nowrap text-muted-foreground">
              {shortDate(endpoint.token_created_at)}
            </span>
          )}
        </div>
      ) : (
        <span title={t("mcp.token.noneHint")}>
          <StatusBadge state="token" tone="muted" label={t("mcp.token.none")} />
        </span>
      )
    },
  }),
  column.accessor("tool_count", {
    header: () => t("mcp.endpoint.tools"),
    cell: ({ row }) => (
      <Button
        variant="link"
        className="h-auto p-0 text-xs whitespace-nowrap text-foreground"
        onClick={() => openDialog({ kind: "mcp-tools", name: row.original.name })}
      >
        {plural(row.original.tool_count, { one: "mcp.tools.one", other: "mcp.tools.other" })}
      </Button>
    ),
  }),
  column.display({
    id: "actions",
    cell: ({ row }) => {
      const endpoint = row.original
      return (
        <div className="flex items-center justify-end gap-2">
          {!endpoint.has_token && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => attempt(() => newToken(endpoint))}
              aria-label={t("mcp.token.createOf", { name: endpoint.name })}
            >
              <RiKey2Line data-icon="inline-start" />
              {t("mcp.token.create")}
            </Button>
          )}
          <EndpointMenu endpoint={endpoint} />
        </div>
      )
    },
  }),
]

/** MCP endpoints: each gives an agent part of the containers' tools with a token of its own. */
export function McpEndpointsPage() {
  const settings = useSettings().data
  const containers = useContainers().data
  const endpoints = useMcpEndpoints()
  // Kept for the endpoint form, whose new containers start with the default scopes.
  useMcpScopes()
  const data = React.useMemo<Row[]>(() => {
    const present = new Set((containers ?? []).map((container) => container.id))
    return (endpoints.data ?? []).map((endpoint) => ({
      ...endpoint,
      url: endpointUrl(location.origin, endpoint.path),
      present,
    }))
  }, [endpoints.data, containers])
  const table = useDataTable({
    data,
    columns,
    getRowId: (row) => row.name,
    sorting: [{ id: "name", desc: false }],
  })

  return (
    <PageBody>
      <DockerAlert />
      {settings?.root_mcp_enabled === false && (
        <div className="px-4 lg:px-6">
          <Alert>
            <RiInformationLine />
            <AlertTitle>{t("mcp.rootOff")}</AlertTitle>
            <AlertDescription>{t("mcp.rootOffHint")}</AlertDescription>
          </Alert>
        </div>
      )}
      <SectionHeading
        title={t("mcp.endpoints.title")}
        description={rich("mcp.endpoints.description", {
          admin: <code className="font-mono text-foreground">/mcp</code>,
        })}
      >
        <Button
          variant="outline"
          size="sm"
          disabled={settings?.root_mcp_enabled === false}
          onClick={() => openDialog({ kind: "mcp-connect" })}
        >
          <RiShareForwardLine data-icon="inline-start" />
          {t("mcp.connect.admin")}
        </Button>
        <Button size="sm" onClick={() => openDialog({ kind: "mcp-endpoint" })}>
          <RiAddLine data-icon="inline-start" />
          {t("mcp.endpoint.new")}
        </Button>
      </SectionHeading>
      <div className="flex flex-col gap-4 overflow-auto px-4 lg:px-6">
        <DataTableView
          table={table}
          empty={
            endpoints.isPending
              ? t("mcp.endpoints.loading")
              : endpoints.error
                ? endpoints.error.message
                : t("mcp.endpoints.none")
          }
        />
        <DataTablePagination table={table} />
      </div>
    </PageBody>
  )
}
