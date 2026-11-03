import * as React from "react"

import { ConceptIcon } from "@/components/concept"
import { Badge } from "@/components/ui/badge"
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
import { runJob } from "@/lib/actions"
import { duplicateName, duplicateVolumes, nameTaken } from "@/lib/duplicate"
import { t } from "@/lib/i18n"
import { pendingContainers } from "@/lib/pending"
import { useContainers, useJobs, useVolumes } from "@/lib/queries"
import { useSubmit } from "./container-dialogs"
import type { DialogProps } from "./side-drawer"
import { Footer } from "./simple-dialogs"

/**
 * Duplicate a container under a new name, with what happens to each volume it mounts: a volume
 * created for it is copied for the new container, every other one is mounted by both.
 */
export function DuplicateDialog({ id, ...props }: DialogProps & { id: string }) {
  const containers = useContainers().data ?? []
  const item = containers.find((entry) => entry.id === id)
  const volumes = useVolumes().data ?? []
  // A name a creation or another duplicate is still making counts as taken too.
  const making = pendingContainers(useJobs(false).data ?? [], containers)
  const known = [...containers, ...making.map((pending) => ({ spec: { name: pending.name } }))]
  const [name, setName] = React.useState(() => (item ? duplicateName(item.spec.name, known) : ""))
  const [start, setStart] = React.useState(true)
  const [endpoint, setEndpoint] = React.useState(true)
  const { busy, error, submit } = useSubmit()
  const field = React.useId()
  if (!item) return null
  const taken = nameTaken(name, known)
  const { copied, shared } = duplicateVolumes(item, volumes)
  const rows = [
    ...copied.map((mount) => ({ mount, copied: true })),
    ...shared.map((mount) => ({ mount, copied: false })),
  ]
  return (
    <Dialog {...props}>
      <DialogContent className="sm:max-w-lg">
        <form
          className="flex flex-col gap-4"
          onSubmit={submit(() =>
            runJob(`containers/${id}/duplicate`, { name, start, mcp_endpoint: endpoint })
          )}
        >
          <DialogHeader>
            <DialogTitle>{t("duplicate.titleOf", { name: item.spec.name })}</DialogTitle>
            <DialogDescription>{t("duplicate.description")}</DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field data-invalid={taken || undefined}>
              <FieldLabel htmlFor={`${field}-name`}>{t("table.name")}</FieldLabel>
              <Input
                id={`${field}-name`}
                required
                maxLength={100}
                aria-invalid={taken || undefined}
                value={name}
                onChange={(event) => setName(event.target.value)}
                autoFocus
              />
              {taken && <FieldError>{t("duplicate.nameTaken", { name: name.trim() })}</FieldError>}
            </Field>
            <Field orientation="horizontal" className="items-start">
              <Checkbox
                id={`${field}-start`}
                checked={start}
                onCheckedChange={(checked) => setStart(checked === true)}
              />
              <FieldContent>
                <FieldLabel htmlFor={`${field}-start`}>{t("duplicate.start")}</FieldLabel>
                <FieldDescription className="text-xs">{t("duplicate.startHint")}</FieldDescription>
              </FieldContent>
            </Field>
            <Field orientation="horizontal" className="items-start">
              <Checkbox
                id={`${field}-endpoint`}
                checked={endpoint}
                onCheckedChange={(checked) => setEndpoint(checked === true)}
              />
              <FieldContent>
                <FieldLabel htmlFor={`${field}-endpoint`}>{t("create.mcpEndpoint")}</FieldLabel>
                <FieldDescription className="text-xs">
                  {t("duplicate.endpointHint")}
                </FieldDescription>
              </FieldContent>
            </Field>
            {/* A fieldset is as wide as its content otherwise, so a long name would not truncate. */}
            <FieldSet className="min-w-0">
              <FieldLegend variant="label">{t("duplicate.volumes")}</FieldLegend>
              {rows.length ? (
                <ul className="flex flex-col gap-2 text-sm">
                  {rows.map(({ mount, copied }) => (
                    <li key={mount.volume} className="flex min-w-0 items-start gap-2">
                      <ConceptIcon concept="volume" className="mt-0.5 size-4 shrink-0" />
                      <span className="flex min-w-0 flex-1 flex-col">
                        <span className="truncate font-mono text-xs" title={mount.volume}>
                          {mount.volume}
                        </span>
                        <span
                          className="truncate text-xs text-muted-foreground"
                          title={mount.target}
                        >
                          {mount.target}
                        </span>
                      </span>
                      <Badge variant={copied ? "secondary" : "outline"} className="shrink-0">
                        {copied ? t("duplicate.copied") : t("duplicate.shared")}
                      </Badge>
                    </li>
                  ))}
                </ul>
              ) : (
                <FieldDescription>{t("duplicate.noVolumes")}</FieldDescription>
              )}
              <FieldDescription className="text-xs">{t("duplicate.notCopied")}</FieldDescription>
            </FieldSet>
            <FieldError>{error}</FieldError>
          </FieldGroup>
          <Footer busy={busy} disabled={taken} label={t("containers.duplicate")} />
        </form>
      </DialogContent>
    </Dialog>
  )
}
