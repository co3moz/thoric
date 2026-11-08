import * as React from "react"
import { RiAlertLine } from "@remixicon/react"
import { toast } from "sonner"

import { CopyButton } from "@/components/copy-button"
import { Alert, AlertDescription } from "@/components/ui/alert"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
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
import { Field, FieldError, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { closeDialog, openDialog } from "@/lib/dialogs"
import { plural, t } from "@/lib/i18n"
import { endpointUrl, toolGroups } from "@/lib/mcp"
import { createToken, deleteEndpoint, removeServer, revokeToken } from "@/lib/mcp-api"
import { useContainers, useEndpointTools, useMcpEndpoints, useSkills } from "@/lib/queries"
import { skillServer, skillsFor } from "@/lib/skills"
import type { EndpointToken } from "@/lib/types"
import { ClientSetup } from "./connect-dialog"
import { useSubmit } from "./container-dialogs"
import type { DialogProps } from "./side-drawer"

/** A value to copy as a whole, such as the token: a read-only field and its copy button. */
function CopyField({ label, value }: { label: string; value: string }) {
  const id = React.useId()
  return (
    <Field>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <div className="flex items-center gap-2">
        <Input
          id={id}
          readOnly
          value={value}
          onFocus={(event) => event.target.select()}
          className="font-mono text-xs"
        />
        <CopyButton text={value} label={t("common.copyOf", { name: label })} />
      </div>
    </Field>
  )
}

/** A new token with what a client needs to connect, shown this one time. */
export function TokenDialog({ token, ...props }: DialogProps & { token: EndpointToken }) {
  const { endpoint } = token
  const url = endpointUrl(location.origin, endpoint.path)
  // The one moment the token shows is when a client is set up, skills included, as Connect does.
  const skills = skillsFor(useSkills().data ?? [], skillServer(endpoint.name, endpoint))
  return (
    <Dialog {...props}>
      {/* A click beside it or Escape must not throw the only copy of the token away. */}
      <DialogContent
        className="flex max-h-[calc(100dvh-2rem)] flex-col sm:max-w-2xl"
        onInteractOutside={(event) => event.preventDefault()}
        onEscapeKeyDown={(event) => event.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>{t("mcp.token.titleOf", { name: endpoint.name })}</DialogTitle>
          <DialogDescription>{t("mcp.token.description")}</DialogDescription>
        </DialogHeader>
        <div className="-mx-4 flex min-h-0 flex-col gap-4 overflow-y-auto px-4 py-1">
          <Alert>
            <RiAlertLine className="text-warning" />
            <AlertDescription>{t("mcp.token.once")}</AlertDescription>
          </Alert>
          <CopyField label={t("mcp.token.token")} value={token.token} />
          <CopyField label={t("mcp.endpoint.url")} value={url} />
          <ClientSetup
            target={{ name: endpoint.name, url, path: endpoint.path, token: token.token }}
            skills={skills}
          />
        </div>
        <DialogFooter>
          <DialogClose asChild>
            <Button type="button">{t("common.done")}</Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** Replaces or revokes an endpoint's token, or deletes the endpoint, once confirmed. */
export function EndpointConfirmDialog({
  name,
  action,
  ...props
}: DialogProps & { name: string; action: "replace" | "revoke" | "delete" }) {
  const { busy, error, submit } = useSubmit()
  const run = async () => {
    if (action === "replace") {
      openDialog({ kind: "mcp-token", token: await createToken(name) })
      return
    }
    if (action === "revoke") await revokeToken(name)
    else await deleteEndpoint(name)
    closeDialog()
    toast.success(t(action === "revoke" ? "mcp.token.revoked" : "mcp.endpoint.deleted", { name }))
  }
  const text = {
    replace: {
      title: "mcp.token.replaceTitle",
      description: "mcp.token.replaceDescription",
      button: "mcp.token.replace",
    },
    revoke: {
      title: "mcp.token.revokeTitle",
      description: "mcp.token.revokeDescription",
      button: "mcp.token.revoke",
    },
    delete: {
      title: "mcp.endpoint.deleteTitle",
      description: "mcp.endpoint.deleteDescription",
      button: "common.delete",
    },
  } as const
  return (
    <AlertDialog {...props}>
      <AlertDialogContent>
        <form className="contents" onSubmit={submit(run)}>
          <AlertDialogHeader>
            <AlertDialogTitle className="break-all">
              {t(text[action].title, { name })}
            </AlertDialogTitle>
            <AlertDialogDescription>{t(text[action].description)}</AlertDialogDescription>
          </AlertDialogHeader>
          <FieldError>{error}</FieldError>
          <AlertDialogFooter>
            <AlertDialogCancel type="button">{t("common.cancel")}</AlertDialogCancel>
            <Button type="submit" variant="destructive" disabled={busy}>
              {busy && <Spinner data-icon="inline-start" />}
              {t(text[action].button)}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** The tools an endpoint exposes, grouped by the container and server they come from. */
export function ToolsDialog({ name, ...props }: DialogProps & { name: string }) {
  const tools = useEndpointTools(name)
  const endpoint = useMcpEndpoints(false).data?.find((entry) => entry.name === name)
  const containers = useContainers().data ?? []
  // A tool of the endpoint itself, such as get_job, belongs to no container.
  const containerName = (id: string | null) =>
    id === null
      ? t("mcp.tools.endpoint")
      : (endpoint?.grants.find((grant) => grant.container_id === id)?.container_name ??
        containers.find((container) => container.id === id)?.spec.name ??
        id.slice(0, 12))
  const groups = toolGroups(tools.data ?? [])
  return (
    <Dialog {...props}>
      <DialogContent className="flex max-h-[85vh] flex-col sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle className="break-all">{t("mcp.tools.titleOf", { name })}</DialogTitle>
          <DialogDescription>
            {tools.data
              ? plural(tools.data.length, {
                  one: "mcp.tools.description.one",
                  other: "mcp.tools.description.other",
                })
              : t("mcp.tools.loading")}
          </DialogDescription>
        </DialogHeader>
        <div className="-mx-4 flex min-h-0 flex-col gap-4 overflow-y-auto px-4 pb-1">
          {tools.error ? (
            <p className="text-sm text-destructive">{tools.error.message}</p>
          ) : !tools.data ? (
            <>
              <Skeleton className="h-6 w-1/2" />
              <Skeleton className="h-32 w-full" />
            </>
          ) : !groups.length ? (
            <p className="text-sm text-muted-foreground">{t("mcp.tools.none")}</p>
          ) : (
            groups.map((group) => (
              <section
                key={`${group.source}-${group.container_id ?? ""}-${group.server ?? ""}`}
                className="flex flex-col"
              >
                <h3 className="flex flex-wrap items-center gap-2 text-sm font-medium">
                  {group.source === "thoric" ? (
                    <Badge variant="outline" className="text-muted-foreground">
                      Thoric
                    </Badge>
                  ) : (
                    <Badge variant="outline" className="font-mono text-muted-foreground">
                      {group.server}
                    </Badge>
                  )}
                  <span className="break-all">{containerName(group.container_id)}</span>
                  <span className="text-xs font-normal text-muted-foreground">
                    {plural(group.tools.length, { one: "mcp.tools.one", other: "mcp.tools.other" })}
                  </span>
                </h3>
                <ul className="flex flex-col divide-y">
                  {group.tools.map((tool) => (
                    <li key={tool.name} className="flex flex-col gap-0.5 py-2">
                      <code className="font-mono text-xs break-all">{tool.name}</code>
                      {tool.description && (
                        <p className="line-clamp-3 text-xs text-muted-foreground">
                          {tool.description}
                        </p>
                      )}
                    </li>
                  ))}
                </ul>
              </section>
            ))
          )}
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

/** Removes a desktop MCP server registered in a container, once confirmed. */
export function ServerRemoveDialog({
  container,
  name,
  ...props
}: DialogProps & { container: string; name: string }) {
  const item = useContainers().data?.find((entry) => entry.id === container)
  const { busy, error, submit } = useSubmit()
  return (
    <AlertDialog {...props}>
      <AlertDialogContent>
        <form
          className="contents"
          onSubmit={submit(async () => {
            await removeServer(container, name)
            closeDialog()
            toast.success(t("mcp.server.removed", { name }))
          })}
        >
          <AlertDialogHeader>
            <AlertDialogTitle className="break-all">
              {item
                ? t("mcp.server.removeTitleIn", { name, container: item.spec.name })
                : t("mcp.server.removeTitle", { name })}
            </AlertDialogTitle>
            <AlertDialogDescription>{t("mcp.server.removeDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          <FieldError>{error}</FieldError>
          <AlertDialogFooter>
            <AlertDialogCancel type="button">{t("common.cancel")}</AlertDialogCancel>
            <Button type="submit" variant="destructive" disabled={busy}>
              {busy && <Spinner data-icon="inline-start" />}
              {t("mcp.server.remove")}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
