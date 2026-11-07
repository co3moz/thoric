import * as React from "react"
import {
  RiAlertLine,
  RiAppsLine,
  RiArrowDownSLine,
  RiCloseLine,
  RiEyeLine,
  RiFileEditLine,
  RiFileTextLine,
  RiFilmLine,
  RiKeyboardLine,
  RiPlugLine,
  RiShutDownLine,
  RiStackLine,
  RiTerminalBoxLine,
  type RemixiconComponentType,
} from "@remixicon/react"
import { cn } from "cn"
import { toast } from "sonner"

import { ConceptIcon } from "@/components/concept"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { closeDialog, openDialog, useDraft } from "@/lib/dialogs"
import { formatNumber, plural, rich, t } from "@/lib/i18n"
import {
  ALIAS_PATTERN,
  defaultAlias,
  ENDPOINT_NAME_PATTERN,
  endpointUrl,
  grantDraft,
  grantRequests,
  newGrant,
  orderScopes,
  scopeDescription,
  scopeLabel,
  SCOPES,
  type GrantDraft,
} from "@/lib/mcp"
import { createEndpoint, updateEndpoint } from "@/lib/mcp-api"
import {
  useContainers,
  useMcpEndpoints,
  useMcpScopes,
  useMcpServers,
  useScaleSets,
} from "@/lib/queries"
import type { ScaleSet } from "@/lib/scale"
import type { ContainerItem, DesktopServer, McpScope, McpScopes, ServerTool } from "@/lib/types"
import { useSubmit } from "./container-dialogs"
import type { DialogProps } from "./side-drawer"
import { Footer } from "./simple-dialogs"

/** What each scope is about, at a glance. */
const SCOPE_ICONS: Record<McpScope, RemixiconComponentType> = {
  view: RiEyeLine,
  input: RiKeyboardLine,
  applications: RiAppsLine,
  files_read: RiFileTextLine,
  files_write: RiFileEditLine,
  commands: RiTerminalBoxLine,
  media: RiFilmLine,
  power: RiShutDownLine,
  scale: RiStackLine,
}
/** A scope's icon; a scope a newer manager added gets a plain one. */
const scopeIcon = (scope: string): RemixiconComponentType =>
  scope in SCOPE_ICONS ? SCOPE_ICONS[scope as McpScope] : RiPlugLine

/**
 * The tools a scope or a desktop server gives the endpoint, behind their count. Open, each has a
 * checkbox, and an unticked one is left out of the endpoint.
 */
function ToolChoices({
  id,
  tools,
  excluded,
  onChange,
}: {
  id: string
  tools: ServerTool[]
  /** Every tool the grant leaves out, not only this list's. */
  excluded: string[]
  onChange: (excluded: string[]) => void
}) {
  const [open, setOpen] = React.useState(false)
  const included = tools.filter((tool) => !excluded.includes(tool.exposed)).length
  const toggle = (name: string, on: boolean) =>
    onChange(on ? excluded.filter((other) => other !== name) : [...excluded, name])
  return (
    <div className="flex flex-col gap-2">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen(!open)}
        className={cn(
          "flex w-fit items-center gap-1 rounded-md text-xs outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50",
          included < tools.length ? "text-foreground" : "text-muted-foreground"
        )}
      >
        <RiArrowDownSLine
          className={cn("size-3.5 shrink-0 transition-transform", open && "rotate-180")}
          aria-hidden
        />
        {included === tools.length
          ? plural(tools.length, { one: "mcp.tools.one", other: "mcp.tools.other" })
          : t("mcp.grant.toolsOf", {
              included: formatNumber(included),
              count: formatNumber(tools.length),
            })}
      </button>
      <ul id={id} hidden={!open} className="flex flex-col gap-2">
        {tools.map((tool) => (
          <li key={tool.exposed}>
            <Label className="flex items-start gap-2 font-normal">
              <Checkbox
                className="mt-0.5"
                checked={!excluded.includes(tool.exposed)}
                onCheckedChange={(checked) => toggle(tool.exposed, checked === true)}
              />
              <span className="flex min-w-0 flex-col gap-0.5 leading-snug">
                <code className="font-mono text-xs break-all">{tool.exposed}</code>
                {tool.description && (
                  <span className="line-clamp-2 text-xs text-muted-foreground">
                    {tool.description}
                  </span>
                )}
              </span>
            </Label>
          </li>
        ))}
      </ul>
    </div>
  )
}

/** One container of an endpoint: the name agents know it by, its scopes and its servers. */
function GrantFields({
  draft,
  onChange,
  onRemove,
  container,
  servers,
  scopes,
}: {
  draft: GrantDraft
  onChange: (draft: GrantDraft) => void
  onRemove: () => void
  /** The container, or nothing once it was deleted. */
  container?: ContainerItem
  /** The container's desktop MCP servers, with the tools the manager last read from each. */
  servers: DesktopServer[]
  scopes?: McpScopes
}) {
  const id = React.useId()
  const set = (patch: Partial<GrantDraft>) => onChange({ ...draft, ...patch })
  const name = draft.set
    ? t("scale.setOf", { name: draft.set })
    : (container?.spec.name ?? draft.container_name)
  // Every scope this release knows, and any a newer manager offers besides; `scale` only for a set.
  const offered = orderScopes([
    ...SCOPES,
    ...(scopes?.scopes.map((scope) => scope.id) ?? []),
  ]).filter((scope) => !!draft.set || scope !== "scale")
  // A container tool keeps its own name on the endpoint.
  const toolsOf = (scope: McpScope): ServerTool[] =>
    (scopes?.scopes.find((entry) => entry.id === scope)?.tools ?? []).map((tool) => ({
      name: tool,
      exposed: tool,
      description: "",
    }))
  // Servers chosen earlier stay listed after they were removed, so they can be taken out.
  const names = [...new Set([...servers.map((server) => server.name), ...draft.servers])]
  const toggleScope = (scope: McpScope, on: boolean) =>
    set({
      scopes: on
        ? orderScopes([...draft.scopes, scope])
        : draft.scopes.filter((other) => other !== scope),
    })
  const toggleServer = (server: string, on: boolean) =>
    set({
      servers: on
        ? names.filter((other) => other === server || draft.servers.includes(other))
        : draft.servers.filter((other) => other !== server),
    })
  const setExcluded = (excluded: string[]) => set({ excluded })

  return (
    <div className="flex flex-col gap-4 rounded-2xl border p-4">
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <ConceptIcon concept="container" className="size-4 shrink-0" />
          <span className="text-sm font-medium break-all">{name}</span>
          {!container && !draft.set && (
            <Badge variant="outline" className="text-warning">
              {t("mcp.grant.deleted")}
            </Badge>
          )}
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label={t("mcp.grant.removeOf", { name })}
          onClick={onRemove}
        >
          <RiCloseLine />
        </Button>
      </div>
      <Field>
        <FieldLabel htmlFor={`${id}-alias`}>{t("mcp.grant.alias")}</FieldLabel>
        <Input
          id={`${id}-alias`}
          required
          pattern={ALIAS_PATTERN}
          title={t("mcp.namePattern")}
          value={draft.alias}
          onChange={(event) => set({ alias: event.target.value })}
          className="font-mono"
        />
        <FieldDescription className="text-xs">{t("mcp.grant.aliasHint")}</FieldDescription>
      </Field>
      <FieldSet className="gap-3">
        <FieldLegend variant="label" className="mb-0">
          {t("mcp.grant.scopes")}
        </FieldLegend>
        {/* One row per scope: its icon, its name and what it covers on one line that wraps, and
            below them, while it is granted, its tools behind their count. */}
        <ul className="flex flex-col divide-y rounded-xl border">
          {offered.map((scope) => {
            const Icon = scopeIcon(scope)
            const on = draft.scopes.includes(scope)
            const tools = toolsOf(scope)
            const description = scopeDescription(scope)
            return (
              <li key={scope} className="px-3 py-2.5">
                <Field orientation="horizontal" className="items-start gap-3">
                  <Checkbox
                    id={`${id}-${scope}`}
                    className="mt-0.5"
                    checked={on}
                    onCheckedChange={(checked) => toggleScope(scope, checked === true)}
                  />
                  <Icon className="mt-px size-4 shrink-0 text-muted-foreground" aria-hidden />
                  <FieldContent className="gap-1.5">
                    <FieldLabel htmlFor={`${id}-${scope}`} className="block w-full leading-snug">
                      <span className="font-medium">{scopeLabel(scope)}</span>
                      {description && (
                        <span className="font-normal text-muted-foreground"> {description}</span>
                      )}
                    </FieldLabel>
                    {on && tools.length > 0 && (
                      <ToolChoices
                        id={`${id}-${scope}-tools`}
                        tools={tools}
                        excluded={draft.excluded}
                        onChange={setExcluded}
                      />
                    )}
                  </FieldContent>
                </Field>
              </li>
            )
          })}
        </ul>
        {draft.scopes.includes("commands") && container?.spec.sudo && (
          <Alert>
            <RiAlertLine className="text-warning" />
            <AlertDescription>{t("mcp.grant.sudoWarning", { name })}</AlertDescription>
          </Alert>
        )}
        <FieldDescription className="text-xs">
          {t("mcp.grant.never")} {t("mcp.grant.toolsHint")}
        </FieldDescription>
      </FieldSet>
      <FieldSet className="gap-3">
        <FieldLegend variant="label" className="mb-0" id={`${id}-servers`}>
          {t("mcp.grant.servers")}
        </FieldLegend>
        <RadioGroup
          aria-labelledby={`${id}-servers`}
          value={draft.all ? "all" : "chosen"}
          onValueChange={(value) => set({ all: value === "all" })}
        >
          <Label className="flex items-center gap-2 font-normal">
            <RadioGroupItem value="all" />
            {t("mcp.grant.allServers")}
          </Label>
          <Label className="flex items-center gap-2 font-normal">
            <RadioGroupItem value="chosen" />
            {t("mcp.grant.chosenServers")}
          </Label>
        </RadioGroup>
        {/* The servers, each with a checkbox when they are chosen here, and under an included one
            its tools as the manager last read them, behind their count. */}
        {names.length ? (
          <ul className="flex flex-col gap-2 pl-6">
            {names.map((server) => {
              const record = servers.find((entry) => entry.name === server)
              const included = draft.all || draft.servers.includes(server)
              return (
                <li key={server} className="flex flex-col gap-1.5">
                  {draft.all ? (
                    <code className="font-mono text-xs leading-none">{server}</code>
                  ) : (
                    <Label className="flex items-center gap-2 font-normal">
                      <Checkbox
                        checked={draft.servers.includes(server)}
                        onCheckedChange={(checked) => toggleServer(server, checked === true)}
                      />
                      <code className="font-mono text-xs">{server}</code>
                    </Label>
                  )}
                  {included &&
                    record &&
                    (record.tools.length ? (
                      <div className="pl-6">
                        <ToolChoices
                          id={`${id}-${server}-tools`}
                          tools={record.tools}
                          excluded={draft.excluded}
                          onChange={setExcluded}
                        />
                      </div>
                    ) : (
                      <p className="pl-6 text-xs text-muted-foreground">
                        {record.state === "ready"
                          ? t("mcp.server.noTools")
                          : t("mcp.server.toolsUnread")}
                      </p>
                    ))}
                </li>
              )
            })}
          </ul>
        ) : (
          !draft.all && (
            <FieldDescription className="pl-6 text-xs">{t("mcp.grant.noServers")}</FieldDescription>
          )
        )}
      </FieldSet>
    </div>
  )
}

/** The picker's value of a scale set, beside the containers' IDs. */
const SET_CHOICE = "set:"

/**
 * The desktop servers a grant chooses from: its container's, or for a scale set those of its
 * instances, each name once, since they come from one image.
 */
function grantServers(grant: GrantDraft, servers: DesktopServer[], sets: ScaleSet[]) {
  if (!grant.set) return servers.filter((server) => server.container_id === grant.container_id)
  const members = new Set(
    sets.find((set) => set.name === grant.set)?.instances.map((entry) => entry.container_id) ?? []
  )
  const seen = new Set<string>()
  return servers.filter((server) => {
    if (!members.has(server.container_id) || seen.has(server.name)) return false
    seen.add(server.name)
    return true
  })
}

/**
 * Creates an MCP endpoint, or changes which containers an existing one reaches and what it may do
 * there. Its name is its URL, so an existing endpoint keeps it.
 */
export function EndpointDialog({ name: existing, ...props }: DialogProps & { name?: string }) {
  const endpoints = useMcpEndpoints(false).data
  // The endpoint as the table showed it; its grants fill the form once.
  const [endpoint] = React.useState(() =>
    existing ? endpoints?.find((entry) => entry.name === existing) : undefined
  )
  const scopes = useMcpScopes().data
  const containers = useContainers().data ?? []
  const servers = useMcpServers().data ?? []
  const sets = useScaleSets().data ?? []
  const [name, setName] = React.useState(existing ?? "")
  const [grants, setGrants] = React.useState<GrantDraft[]>(
    () => endpoint?.grants.map(grantDraft) ?? []
  )
  useDraft({ name, grants })
  const { busy, error, submit } = useSubmit()
  const id = React.useId()
  // A scale set's grant is its endpoint's only one.
  const ofSet = grants.some((grant) => grant.set)
  const available = ofSet
    ? []
    : containers.filter((container) => !grants.some((grant) => grant.container_id === container.id))
  const availableSets = grants.length ? [] : sets
  const add = (containerId: string) => {
    if (containerId.startsWith(SET_CHOICE)) {
      const set = containerId.slice(SET_CHOICE.length)
      setGrants([
        {
          container_id: "",
          set,
          container_name: set,
          alias: defaultAlias(set),
          scopes: orderScopes([...(scopes?.default ?? ["view"]), "scale"]),
          all: true,
          servers: [],
          excluded: [],
        },
      ])
      return
    }
    const container = containers.find((entry) => entry.id === containerId)
    if (!container) return
    // Without the manager's defaults the new container starts with the least: viewing.
    const defaults = scopes?.default ?? ["view"]
    setGrants([
      ...grants,
      newGrant(
        { id: container.id, name: container.spec.name },
        defaults,
        grants.map((grant) => grant.alias)
      ),
    ])
  }
  const save = async () => {
    const body = grantRequests(grants, scopes)
    if (existing) {
      await updateEndpoint(existing, body)
      closeDialog()
      toast.success(t("mcp.endpoint.saved", { name: existing }))
      return
    }
    // The answer holds the token, which is shown once.
    openDialog({ kind: "mcp-token", token: await createEndpoint(name, body) })
  }

  if (existing && !endpoint)
    return (
      <Dialog {...props}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("mcp.endpoint.edit")}</DialogTitle>
            <DialogDescription>{t("mcp.endpoint.gone")}</DialogDescription>
          </DialogHeader>
        </DialogContent>
      </Dialog>
    )
  return (
    <Dialog {...props}>
      {/* The dialog stays inside the window: its body scrolls and the buttons stay in view. */}
      <DialogContent className="flex max-h-[calc(100dvh-2rem)] flex-col sm:max-w-2xl">
        <form className="flex min-h-0 flex-col gap-4" onSubmit={submit(save)}>
          <DialogHeader>
            <DialogTitle>
              {existing ? t("mcp.endpoint.editOf", { name: existing }) : t("mcp.endpoint.new")}
            </DialogTitle>
            <DialogDescription>{t("mcp.endpoint.formDescription")}</DialogDescription>
          </DialogHeader>
          <FieldGroup className="-mx-4 min-h-0 w-auto overflow-y-auto px-4 py-1">
            <Field>
              <FieldLabel htmlFor={`${id}-name`}>{t("table.name")}</FieldLabel>
              <Input
                id={`${id}-name`}
                required
                readOnly={!!existing}
                pattern={ENDPOINT_NAME_PATTERN}
                title={t("mcp.namePattern")}
                placeholder="unity-agent"
                value={name}
                onChange={(event) => setName(event.target.value)}
                className="font-mono"
                autoFocus={!existing}
              />
              <FieldDescription className="text-xs break-all">
                {rich(existing ? "mcp.endpoint.urlFixed" : "mcp.endpoint.urlHint", {
                  url: (
                    <code className="font-mono text-foreground">
                      {endpointUrl(location.origin, `/mcp/${name || "…"}`)}
                    </code>
                  ),
                })}
              </FieldDescription>
            </Field>
            <FieldSet className="gap-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <FieldLegend variant="label" className="mb-0">
                  {t("mcp.endpoint.containers")}
                </FieldLegend>
                <Select
                  value=""
                  onValueChange={add}
                  disabled={!available.length && !availableSets.length}
                >
                  <SelectTrigger
                    size="sm"
                    className="w-full sm:w-64"
                    aria-label={t("mcp.grant.add")}
                  >
                    <SelectValue
                      placeholder={
                        available.length || availableSets.length
                          ? t("mcp.grant.addPlaceholder")
                          : t("mcp.grant.allAdded")
                      }
                    />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectGroup>
                      {available.map((container) => (
                        <SelectItem key={container.id} value={container.id}>
                          <ConceptIcon concept="container" />
                          {container.spec.name}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                    {availableSets.length > 0 && (
                      <SelectGroup>
                        {availableSets.map((set) => (
                          <SelectItem key={set.name} value={`${SET_CHOICE}${set.name}`}>
                            <RiStackLine />
                            {t("scale.setOf", { name: set.name })}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    )}
                  </SelectContent>
                </Select>
              </div>
              {grants.length ? (
                grants.map((grant, index) => (
                  <GrantFields
                    key={grant.set ?? grant.container_id}
                    draft={grant}
                    container={containers.find((entry) => entry.id === grant.container_id)}
                    servers={grantServers(grant, servers, sets)}
                    scopes={scopes}
                    onChange={(next) =>
                      setGrants(grants.map((other, at) => (at === index ? next : other)))
                    }
                    onRemove={() => setGrants(grants.filter((_, at) => at !== index))}
                  />
                ))
              ) : (
                <FieldDescription className="text-xs">{t("mcp.endpoint.addHint")}</FieldDescription>
              )}
            </FieldSet>
          </FieldGroup>
          {/* Outside the scrolling fields, so a long form cannot hide the outcome. */}
          <FieldError>{error}</FieldError>
          <Footer
            busy={busy}
            label={existing ? t("common.save") : t("common.create")}
            disabled={!grants.length}
          />
        </form>
      </DialogContent>
    </Dialog>
  )
}
