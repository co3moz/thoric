import * as React from "react"
import { RiPlugLine } from "@remixicon/react"
import { toast } from "sonner"

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Spinner } from "@/components/ui/spinner"
import { runJob } from "@/lib/actions"
import { api } from "@/lib/api"
import { closeDialog, useDraft } from "@/lib/dialogs"
import { t } from "@/lib/i18n"
import { refreshLists, useContainers, useScaleSets } from "@/lib/queries"
import {
  scalingBody,
  scalingDraft,
  scalingError,
  scalingTargets,
  type ScaleSet,
  type ScalingDraft,
} from "@/lib/scale"
import { useSubmit } from "./container-dialogs"
import { ScalingFields } from "./scaling-fields"
import { Footer } from "./simple-dialogs"
import type { DialogProps } from "./side-drawer"
import { SwitchTile } from "./spec-fields"

/** What a form's scaling draft cannot send, as the manager would say it. */
function checked(draft: ScalingDraft) {
  const problem = scalingError(draft)
  if (problem) throw new Error(t(`scale.error.${problem}`))
}

/** Reads the lists a set's change shows in. */
const refreshed = () => refreshLists("scale-sets", "containers", "mcp")

/**
 * Makes an existing container scalable: it becomes instance 1 of a new scale set whose template
 * is its settings and the compose values it was created with.
 */
export function MakeScalableDialog({ id, ...props }: DialogProps & { id: string }) {
  const item = useContainers().data?.find((entry) => entry.id === id)
  const [draft, setDraft] = React.useState<ScalingDraft>(() => ({
    ...scalingDraft(),
    enabled: true,
  }))
  const [endpoint, setEndpoint] = React.useState(true)
  useDraft({ draft, endpoint })
  const { busy, error, submit } = useSubmit()
  const hint = React.useId()
  if (!item) return null
  const targets = scalingTargets([...item.spec.mounts, ...(item.setup?.mounts ?? [])])
  return (
    <Dialog {...props}>
      <DialogContent className="sm:max-w-xl">
        <form
          className="flex flex-col gap-4"
          onSubmit={submit(async () => {
            checked(draft)
            const set = await api<ScaleSet>("scale-sets", {
              container_id: id,
              scaling: scalingBody(draft),
              mcp_endpoint: endpoint,
            })
            await refreshed()
            closeDialog()
            toast.success(t("scale.made", { name: set.name }))
          })}
        >
          <DialogHeader>
            <DialogTitle>{t("scale.makeTitle", { name: item.spec.name })}</DialogTitle>
            <DialogDescription>{t("scale.makeDescription")}</DialogDescription>
          </DialogHeader>
          <ScalingFields
            draft={draft}
            onChange={setDraft}
            targets={targets}
            name={item.spec.name}
          />
          <Field>
            <SwitchTile
              icon={RiPlugLine}
              label={t("scale.endpoint")}
              checked={endpoint}
              onCheckedChange={setEndpoint}
              describedBy={hint}
            />
            <FieldDescription id={hint} className="text-xs">
              {t("scale.endpointHint")}
            </FieldDescription>
          </Field>
          <FieldError>{error}</FieldError>
          <Footer busy={busy} label={t("scale.make")} />
        </form>
      </DialogContent>
    </Dialog>
  )
}

/** Changes a set's limits and what its new instances get for each volume. */
export function ScaleSetDialog({ name, ...props }: DialogProps & { name: string }) {
  const set = useScaleSets().data?.find((entry) => entry.name === name)
  const [draft, setDraft] = React.useState<ScalingDraft>(() => ({
    ...scalingDraft(set),
    enabled: true,
  }))
  useDraft({ draft })
  const { busy, error, submit } = useSubmit()
  if (!set) return null
  const targets = set.slots.map((slot) => slot.target)
  return (
    <Dialog {...props}>
      <DialogContent className="sm:max-w-xl">
        <form
          className="flex flex-col gap-4"
          onSubmit={submit(async () => {
            checked(draft)
            const { min, max, idle_delete_minutes } = scalingBody(draft)
            await api(
              `scale-sets/${name}`,
              {
                min,
                max,
                idle_delete_minutes,
                volumes: Object.fromEntries(
                  targets.map((target) => [target, draft.volumes[target] ?? "shared"])
                ),
              },
              "PUT"
            )
            await refreshed()
            closeDialog()
            toast.success(t("scale.saved", { name }))
          })}
        >
          <DialogHeader>
            <DialogTitle>{t("scale.settingsTitle", { name })}</DialogTitle>
            <DialogDescription>{t("scale.settingsDescription")}</DialogDescription>
          </DialogHeader>
          <ScalingFields named draft={draft} onChange={setDraft} targets={targets} name={name} />
          <FieldError>{error}</FieldError>
          <Footer busy={busy} label={t("common.save")} />
        </form>
      </DialogContent>
    </Dialog>
  )
}

/** What a new instance gets for a volume: the set's policy, or one of the two for this one. */
type Choice = "policy" | "shared" | "new"

/** Creates an instance of a set, with what it is for and its own choice of volumes. */
export function NewInstanceDialog({ name, ...props }: DialogProps & { name: string }) {
  const set = useScaleSets().data?.find((entry) => entry.name === name)
  const [note, setNote] = React.useState("")
  const [choices, setChoices] = React.useState<Record<string, Choice>>({})
  useDraft({ note, choices })
  const { busy, error, submit } = useSubmit()
  const id = React.useId()
  if (!set) return null
  const full = set.instances.length >= set.max
  return (
    <Dialog {...props}>
      <DialogContent className="sm:max-w-xl">
        <form
          className="flex flex-col gap-4"
          onSubmit={submit(() =>
            runJob(`scale-sets/${name}/instances`, {
              note: note.trim(),
              volumes: Object.fromEntries(
                Object.entries(choices).filter(([, choice]) => choice !== "policy")
              ),
            })
          )}
        >
          <DialogHeader>
            <DialogTitle>{t("scale.newInstanceTitle", { name })}</DialogTitle>
            <DialogDescription>
              {full
                ? t("scale.full", { max: set.max })
                : t("scale.newInstanceDescription", { count: set.instances.length, max: set.max })}
            </DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor={`${id}-note`}>{t("scale.note")}</FieldLabel>
              <Input
                id={`${id}-note`}
                maxLength={200}
                placeholder={t("scale.notePlaceholder")}
                value={note}
                onChange={(event) => setNote(event.target.value)}
              />
            </Field>
            {set.slots.length > 0 && (
              <Field>
                <FieldLabel>{t("scale.volumes")}</FieldLabel>
                <ul className="flex flex-col divide-y rounded-xl border">
                  {set.slots.map((slot) => (
                    <li
                      key={slot.target}
                      className="flex flex-col gap-2 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between"
                    >
                      <div className="flex min-w-0 flex-col">
                        <code className="font-mono text-xs break-all">{slot.target}</code>
                        {slot.label && (
                          <span className="text-xs text-muted-foreground">{slot.label}</span>
                        )}
                      </div>
                      <Select
                        value={choices[slot.target] ?? "policy"}
                        onValueChange={(value) =>
                          setChoices({ ...choices, [slot.target]: value as Choice })
                        }
                      >
                        <SelectTrigger
                          size="sm"
                          className="w-full sm:w-56"
                          aria-label={t("scale.policyOf", { target: slot.target })}
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value="policy">
                              {t("scale.setPolicy", {
                                policy: t(`scale.policy.${slot.policy}`),
                              })}
                            </SelectItem>
                            <SelectItem value="shared">{t("scale.policy.shared")}</SelectItem>
                            <SelectItem value="new">{t("scale.policy.new")}</SelectItem>
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    </li>
                  ))}
                </ul>
              </Field>
            )}
          </FieldGroup>
          <FieldError>{error}</FieldError>
          <Footer busy={busy} label={t("scale.addInstance")} disabled={full} />
        </form>
      </DialogContent>
    </Dialog>
  )
}

/** Asks before a set is deleted with its instances and the volumes made for them. */
export function ScaleSetDeleteDialog({ name, ...props }: DialogProps & { name: string }) {
  const set = useScaleSets().data?.find((entry) => entry.name === name)
  const { busy, error, submit } = useSubmit()
  return (
    <AlertDialog {...props}>
      <AlertDialogContent>
        <form
          className="contents"
          onSubmit={submit(() => runJob(`scale-sets/${name}`, undefined, "DELETE"))}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t("scale.deleteTitle", { name })}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("scale.deleteDescription", { count: set?.instances.length ?? 0 })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <FieldError>{error}</FieldError>
          <AlertDialogFooter>
            <AlertDialogCancel type="button">{t("common.cancel")}</AlertDialogCancel>
            <Button type="submit" variant="destructive" disabled={busy}>
              {busy && <Spinner data-icon="inline-start" />}
              {t("common.delete")}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
