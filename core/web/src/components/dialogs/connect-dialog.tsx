import * as React from "react"
import { RiExternalLinkLine, RiInformationLine } from "@remixicon/react"

import { CodeView } from "@/components/code-textarea"
import { CopyButton } from "@/components/copy-button"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { rich, t } from "@/lib/i18n"
import { endpointUrl } from "@/lib/mcp"
import {
  ADMIN_TOKEN,
  ENDPOINT_TOKEN,
  MCP_CLIENTS,
  mcpClient,
  type ClientId,
  type ConnectTarget,
  type McpClient,
  type Snippet,
} from "@/lib/mcp-connect"
import { useConfiguration, useMcpEndpoints, useSkills } from "@/lib/queries"
import { skillCommands, skillServer, skillsFor, type Skill } from "@/lib/skills"
import type { DialogProps } from "./side-drawer"

const CLIENT_KEY = "thoric.mcp.client"

/** A client's name; `generic` is the one that is not a product. */
const clientName = (client: McpClient) =>
  client.id === "generic" ? t("mcp.connect.generic") : client.name

/** The client picked last time, a convenience kept in this browser only. */
function storedClient(): ClientId {
  try {
    return mcpClient(localStorage.getItem(CLIENT_KEY)).id
  } catch {
    return MCP_CLIENTS[0].id
  }
}

function SnippetBlock({ snippet }: { snippet: Snippet }) {
  const label = t(`mcp.connect.snippet.${snippet.kind}`)
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <div className="flex items-end justify-between gap-2">
        <div className="flex min-w-0 flex-col gap-1">
          <span className="text-sm font-medium">{label}</span>
          {snippet.paths && (
            <span className="flex flex-wrap gap-x-2 gap-y-0.5 text-xs text-muted-foreground">
              {snippet.paths.map((path) => (
                <code key={path} className="font-mono break-all">
                  {path}
                </code>
              ))}
            </span>
          )}
        </div>
        <CopyButton text={snippet.text} label={t("common.copyOf", { name: label })} />
      </div>
      <CodeView
        language={snippet.language}
        value={snippet.text}
        label={label}
        // The dialog's body scrolls; a second scroll inside it would hide the end of an entry.
        className="max-h-none break-words whitespace-pre-wrap"
      />
    </div>
  )
}

/**
 * The skills an agent of this server should have, for `client`: each one's link, the client's
 * skills folder and the commands that copy them there from this dashboard's address.
 */
function SkillsSetup({ client, skills }: { client: McpClient; skills: Skill[] }) {
  const id = React.useId()
  const folder = client.skills
  const code = (path: string) => <code className="font-mono text-xs text-foreground">{path}/</code>
  return (
    <section aria-labelledby={`${id}-skills`} className="flex min-w-0 flex-col gap-3 border-t pt-4">
      <div className="flex flex-col gap-1">
        <h3 id={`${id}-skills`} className="text-sm font-medium">
          {t("mcp.connect.skills.title")}
        </h3>
        <p className="text-sm text-muted-foreground">{t("mcp.connect.skills.description")}</p>
      </div>
      <ul className="flex min-w-0 flex-col gap-2">
        {skills.map((skill) => (
          <li key={skill.name} className="flex min-w-0 flex-col gap-0.5">
            <a
              href={skill.url}
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-1 self-start font-mono text-xs break-all underline-offset-4 hover:underline"
            >
              {skill.name}
              <RiExternalLinkLine className="size-3.5 shrink-0" aria-hidden />
            </a>
            <span className="line-clamp-2 text-xs text-muted-foreground" title={skill.description}>
              {skill.description}
            </span>
          </li>
        ))}
      </ul>
      {folder ? (
        <>
          <p className="text-sm text-muted-foreground">
            {rich("mcp.connect.skills.folder", {
              client: clientName(client),
              user: code(folder.user),
              project: code(folder.project),
            })}
          </p>
          {skillCommands(folder, location.origin, skills).map((snippet) => (
            <SnippetBlock key={`${client.id}-${snippet.kind}`} snippet={snippet} />
          ))}
        </>
      ) : (
        <p className="text-sm text-muted-foreground">
          {t("mcp.connect.skills.noFolder", { client: clientName(client) })}
        </p>
      )}
    </section>
  )
}

/**
 * What each MCP client needs to reach `target`: a choice of client, then the link, the command or
 * the settings to copy, and the skills the server's agents should have, when it offers any.
 * Everything is made here, in the browser; nothing is sent to the manager.
 */
export function ClientSetup({ target, skills = [] }: { target: ConnectTarget; skills?: Skill[] }) {
  const id = React.useId()
  const [picked, setPicked] = React.useState(storedClient)
  const client = mcpClient(picked)
  const pick = (value: string) => {
    if (!value) return
    setPicked(value as ClientId)
    try {
      localStorage.setItem(CLIENT_KEY, value)
    } catch {
      // The choice is a convenience; without storage the first client opens next time.
    }
  }
  const install = client.install?.(target)
  return (
    <div className="flex min-w-0 flex-col gap-4">
      <div className="flex flex-col gap-2">
        <span id={`${id}-client`} className="text-sm font-medium">
          {t("mcp.connect.client")}
        </span>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          spacing={1.5}
          value={client.id}
          onValueChange={pick}
          aria-labelledby={`${id}-client`}
          className="w-full flex-wrap"
        >
          {MCP_CLIENTS.map((entry) => (
            <ToggleGroupItem
              key={entry.id}
              value={entry.id}
              className="data-[state=on]:border-primary data-[state=on]:text-primary"
            >
              {clientName(entry)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
      <p className="text-sm text-muted-foreground">
        {rich(
          `mcp.connect.hint.${client.id}`,
          Object.fromEntries(
            Object.entries(client.terms ?? {}).map(([name, term]) => [
              name,
              <code className="font-mono text-xs text-foreground">{term}</code>,
            ])
          )
        )}
      </p>
      {install && (
        <Button asChild variant="outline" size="sm" className="self-start">
          <a href={install}>
            <RiExternalLinkLine data-icon="inline-start" />
            {t("mcp.connect.install", { client: clientName(client) })}
          </a>
        </Button>
      )}
      {client.snippets(target).map((snippet) => (
        <SnippetBlock key={`${client.id}-${snippet.kind}`} snippet={snippet} />
      ))}
      <a
        href={client.docs}
        target="_blank"
        rel="noreferrer"
        className="inline-flex items-center gap-1 self-start text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
      >
        {client.id === "generic"
          ? t("mcp.connect.docsGeneric")
          : t("mcp.connect.docs", { client: client.name })}
        <RiExternalLinkLine className="size-3.5" aria-hidden />
      </a>
      {skills.length > 0 && <SkillsSetup client={client} skills={skills} />}
    </div>
  )
}

/**
 * Connects an agent to an endpoint, or to Thoric MCP at `/mcp` without a name. The token is
 * shown only once when it is made, so it is typed or pasted here and fills in the settings alone.
 */
export function ConnectDialog({ name, ...props }: DialogProps & { name?: string }) {
  const id = React.useId()
  const endpoint = useMcpEndpoints(false).data?.find((entry) => entry.name === name)
  const skills = skillsFor(useSkills().data ?? [], skillServer(name, endpoint))
  const [token, setToken] = React.useState("")
  const [header, setHeader] = React.useState(true)
  const path = name ? (endpoint?.path ?? `/mcp/${name}`) : "/mcp"
  const manager = useConfiguration().data?.variables.find(
    (variable) => variable.name === "MANAGER_CONTAINER"
  )
  const target: ConnectTarget = {
    name: name ?? "thoric",
    url: endpointUrl(location.origin, path),
    path,
    token: header ? token.trim() || (name ? ENDPOINT_TOKEN : ADMIN_TOKEN) : null,
    container:
      manager && !manager.secret && typeof manager.value === "string" ? manager.value : undefined,
  }
  const tokenHint = !name
    ? t("mcp.connect.tokenAdmin")
    : endpoint && !endpoint.has_token
      ? t("mcp.connect.tokenNone")
      : t("mcp.connect.tokenEndpoint")
  return (
    <Dialog {...props}>
      <DialogContent className="flex max-h-[calc(100dvh-2rem)] flex-col sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle className="break-all">
            {name ? t("mcp.connect.titleOf", { name }) : t("mcp.connect.titleAdmin")}
          </DialogTitle>
          <DialogDescription>{t("mcp.connect.description")}</DialogDescription>
        </DialogHeader>
        <div className="-mx-4 flex min-h-0 flex-col gap-4 overflow-y-auto px-4 py-1">
          {!name && (
            <Alert>
              <RiInformationLine />
              <AlertDescription>{t("mcp.connect.adminWarning")}</AlertDescription>
            </Alert>
          )}
          <Field>
            <div className="flex items-center justify-between gap-3">
              <FieldLabel htmlFor={`${id}-token`}>{t("mcp.token.token")}</FieldLabel>
              <div className="flex items-center gap-2">
                <Switch
                  id={`${id}-header`}
                  size="sm"
                  checked={header}
                  onCheckedChange={setHeader}
                />
                <label htmlFor={`${id}-header`} className="text-xs text-muted-foreground">
                  {t("mcp.connect.header")}
                </label>
              </div>
            </div>
            <Input
              id={`${id}-token`}
              value={token}
              disabled={!header}
              autoComplete="off"
              spellCheck={false}
              placeholder={name ? ENDPOINT_TOKEN : ADMIN_TOKEN}
              onChange={(event) => setToken(event.target.value)}
              className="font-mono text-xs"
            />
            <FieldDescription>{header ? tokenHint : t("mcp.connect.noHeader")}</FieldDescription>
          </Field>
          <ClientSetup target={target} skills={skills} />
        </div>
        <DialogFooter>
          <DialogClose asChild>
            <Button type="button" variant="outline">
              {t("common.close")}
            </Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
