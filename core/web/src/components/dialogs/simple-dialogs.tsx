import * as React from "react"
import { RiAlertLine } from "@remixicon/react"
import { toast } from "sonner"

import { ConceptIcon } from "@/components/concept"
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
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Spinner } from "@/components/ui/spinner"
import { Textarea } from "@/components/ui/textarea"
import { runJob } from "@/lib/actions"
import { api } from "@/lib/api"
import { closeDialog } from "@/lib/dialogs"
import { deletedNotice, imageHolders } from "@/lib/format"
import { formatList, t } from "@/lib/i18n"
import { refreshLists, useContainers, useImages, useVolumes } from "@/lib/queries"
import { ownedVolumes, volumeApi, volumeHolders } from "@/lib/volume-files"
import { useSubmit } from "./container-dialogs"
import type { DialogProps } from "./side-drawer"

export function Footer({
  busy,
  label,
  disabled,
  children,
}: {
  busy: boolean
  label: string
  disabled?: boolean
  /** Secondary actions, shown before Cancel. */
  children?: React.ReactNode
}) {
  return (
    <DialogFooter>
      {children}
      <DialogClose asChild>
        <Button type="button" variant="outline">
          {t("common.cancel")}
        </Button>
      </DialogClose>
      <Button type="submit" disabled={busy || disabled}>
        {busy && <Spinner data-icon="inline-start" />}
        {label}
      </Button>
    </DialogFooter>
  )
}

export function SnapshotDialog({ id, ...props }: DialogProps & { id: string }) {
  const item = useContainers().data?.find((entry) => entry.id === id)
  const [name, setName] = React.useState("")
  const [description, setDescription] = React.useState("")
  const { busy, error, submit } = useSubmit()
  const field = React.useId()
  return (
    <Dialog {...props}>
      <DialogContent className="sm:max-w-md">
        <form
          className="flex flex-col gap-4"
          onSubmit={submit(() => runJob(`containers/${id}/snapshot`, { name, description }))}
        >
          <DialogHeader>
            <DialogTitle>{t("snapshot.title")}</DialogTitle>
            <DialogDescription>
              {item
                ? t("snapshot.descriptionOf", { name: item.spec.name })
                : t("snapshot.description")}
            </DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor={`${field}-name`}>{t("table.name")}</FieldLabel>
              <Input
                id={`${field}-name`}
                required
                maxLength={100}
                value={name}
                onChange={(event) => setName(event.target.value)}
                autoFocus
              />
            </Field>
            <Field>
              <FieldLabel htmlFor={`${field}-description`}>{t("common.description")}</FieldLabel>
              <Textarea
                id={`${field}-description`}
                rows={3}
                maxLength={4096}
                value={description}
                onChange={(event) => setDescription(event.target.value)}
              />
              <FieldDescription>{t("snapshot.hint")}</FieldDescription>
            </Field>
            <FieldError>{error}</FieldError>
          </FieldGroup>
          <Footer busy={busy} label={t("snapshot.title")} />
        </form>
      </DialogContent>
    </Dialog>
  )
}

export function DeleteDialog({ id, ...props }: DialogProps & { id: string }) {
  const item = useContainers().data?.find((entry) => entry.id === id)
  const owned = ownedVolumes(useVolumes().data ?? [], id)
  // Volumes stay unless this is turned on, and it offers only those created for the container.
  const [volumes, setVolumes] = React.useState(false)
  const removing = volumes && owned.length > 0
  const { busy, error, submit } = useSubmit()
  const field = React.useId()
  return (
    <AlertDialog {...props}>
      <AlertDialogContent>
        <form
          className="contents"
          onSubmit={submit(() =>
            runJob(`containers/${id}${removing ? "?delete_volumes=true" : ""}`, undefined, "DELETE")
          )}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>
              {item ? t("delete.titleOf", { name: item.spec.name }) : t("delete.title")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {removing ? t("delete.descriptionVolumes") : t("delete.description")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {owned.length > 0 && (
            <Field orientation="horizontal" className="items-start">
              <Checkbox
                id={field}
                checked={volumes}
                onCheckedChange={(checked) => setVolumes(checked === true)}
              />
              <div className="flex min-w-0 flex-col gap-1">
                <FieldLabel htmlFor={field}>{t("delete.volumes")}</FieldLabel>
                <FieldDescription className="font-mono text-xs break-all">
                  {owned.join(", ")}
                </FieldDescription>
              </div>
            </Field>
          )}
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

/**
 * Asks before a reset, which creates the container again from the image it started from and
 * keeps its volumes, settings and identity, with the volumes named so it is clear what stays.
 */
export function ResetDialog({ id, ...props }: DialogProps & { id: string }) {
  const item = useContainers().data?.find((entry) => entry.id === id)
  const volumes = [
    ...new Set([...(item?.spec.mounts ?? []), ...(item?.setup?.mounts ?? [])].map((m) => m.volume)),
  ]
  const { busy, error, submit } = useSubmit()
  return (
    <AlertDialog {...props}>
      <AlertDialogContent>
        <form className="contents" onSubmit={submit(() => runJob(`containers/${id}/reset`, {}))}>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {item ? t("reset.titleOf", { name: item.spec.name }) : t("reset.title")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("reset.description")}
              {item?.setup && ` ${t("reset.setup")}`} {t("reset.snapshot")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {/* Centered with the header on a phone, as the header is. */}
          {item &&
            (volumes.length > 0 ? (
              <div className="flex flex-col gap-1 text-center sm:text-left">
                <p className="flex items-center justify-center gap-1.5 text-sm font-medium sm:justify-start">
                  <ConceptIcon concept="volume" className="size-4 shrink-0" />
                  {t("reset.volumes")}
                </p>
                <p className="font-mono text-xs break-all text-muted-foreground">
                  {volumes.join(", ")}
                </p>
              </div>
            ) : (
              <p className="text-center text-sm text-muted-foreground sm:text-left">
                {t("reset.noVolumes")}
              </p>
            ))}
          <FieldError>{error}</FieldError>
          <AlertDialogFooter>
            <AlertDialogCancel type="button">{t("common.cancel")}</AlertDialogCancel>
            <Button type="submit" variant="destructive" disabled={busy}>
              {busy && <Spinner data-icon="inline-start" />}
              {t("reset.confirm")}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** Copies a volume's files into a new volume; the copy's job opens once it is queued. */
export function VolumeCopyDialog({ name: source, ...props }: DialogProps & { name: string }) {
  const [name, setName] = React.useState(`${source}-copy`)
  const { busy, error, submit } = useSubmit()
  const field = React.useId()
  return (
    <Dialog {...props}>
      <DialogContent className="sm:max-w-md">
        <form
          className="flex flex-col gap-4"
          onSubmit={submit(() => runJob(`volumes/${encodeURIComponent(source)}/copy`, { name }))}
        >
          <DialogHeader>
            <DialogTitle className="break-all">
              {t("volumes.copyTitle", { name: source })}
            </DialogTitle>
            <DialogDescription>{t("volumes.copyDescription")}</DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor={`${field}-name`}>{t("volumes.copyName")}</FieldLabel>
              <Input
                id={`${field}-name`}
                required
                pattern="[a-zA-Z0-9][a-zA-Z0-9_.\-]+"
                title={t("volumes.namePattern")}
                value={name}
                onChange={(event) => setName(event.target.value)}
                className="font-mono"
                autoFocus
              />
            </Field>
            <FieldError>{error}</FieldError>
          </FieldGroup>
          <Footer busy={busy} label={t("volumes.copy")} />
        </form>
      </DialogContent>
    </Dialog>
  )
}

/** Deletes a volume and its files once confirmed. A volume a container mounts stays, as the
 * manager refuses it too, so the dialog names those containers instead of offering Delete. */
export function VolumeDeleteDialog({ name, ...props }: DialogProps & { name: string }) {
  const holders = volumeHolders(name, useContainers().data ?? [])
  const { busy, error, submit } = useSubmit()
  return (
    <AlertDialog {...props}>
      <AlertDialogContent>
        <form
          className="contents"
          onSubmit={submit(async () => {
            await api(volumeApi(name), undefined, "DELETE")
            closeDialog()
            toast.success(t("volumes.deleted", { name }))
            void refreshLists("volumes")
          })}
        >
          <AlertDialogHeader>
            <AlertDialogTitle className="break-all">
              {t("volumes.deleteTitle", { name })}
            </AlertDialogTitle>
            <AlertDialogDescription>{t("volumes.deleteDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          {holders.length > 0 && (
            <Alert>
              <RiAlertLine className="text-warning" />
              <AlertDescription>
                {t("volumes.deleteMounted", { containers: formatList(holders) })}
              </AlertDescription>
            </Alert>
          )}
          <FieldError>{error}</FieldError>
          <AlertDialogFooter>
            <AlertDialogCancel type="button">{t("common.cancel")}</AlertDialogCancel>
            <Button type="submit" variant="destructive" disabled={busy || holders.length > 0}>
              {busy && <Spinner data-icon="inline-start" />}
              {t("common.delete")}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** Deletes an image once confirmed: a saved image with its record, an import's record alone, or a
 * catalog image's download. An image a container was created from stays, as the manager refuses
 * it too, so the dialog names those containers instead of offering Delete. */
export function ImageDeleteDialog({ id, ...props }: DialogProps & { id: string }) {
  const images = useImages()
  // The image as the row showed it, so the closing dialog keeps its name once the list drops it.
  const [image] = React.useState(() => images.data?.find((item) => item.id === id))
  const containers = useContainers().data ?? []
  const holders = image ? imageHolders(image, containers, images.data ?? []) : []
  const name = image?.name ?? id
  const { busy, error, submit } = useSubmit()
  return (
    <AlertDialog {...props}>
      <AlertDialogContent>
        <form
          className="contents"
          onSubmit={submit(async () => {
            const result = await api<{ image_removed: boolean }>(
              `images/${encodeURIComponent(id)}`,
              undefined,
              "DELETE"
            )
            closeDialog()
            const notice = image ? deletedNotice(image, result.image_removed) : "images.deleted"
            toast.success(t(notice, { name }))
            void refreshLists("images")
          })}
        >
          <AlertDialogHeader>
            <AlertDialogTitle className="break-all">
              {t("images.deleteTitle", { name })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {image?.kind === "catalog"
                ? t("images.deleteCatalog")
                : image?.kind === "import"
                  ? t("images.deleteImport")
                  : t("images.deleteSaved")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {holders.length > 0 && (
            <Alert>
              <RiAlertLine className="text-warning" />
              <AlertDescription>
                {t("images.deleteUsed", { containers: formatList(holders) })}
              </AlertDescription>
            </Alert>
          )}
          <FieldError>{error}</FieldError>
          <AlertDialogFooter>
            <AlertDialogCancel type="button">{t("common.cancel")}</AlertDialogCancel>
            <Button type="submit" variant="destructive" disabled={busy || holders.length > 0}>
              {busy && <Spinner data-icon="inline-start" />}
              {t("common.delete")}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}

export function VolumeDialog(props: DialogProps) {
  const [name, setName] = React.useState("")
  const { busy, error, submit } = useSubmit()
  const field = React.useId()
  return (
    <Dialog {...props}>
      <DialogContent className="sm:max-w-md">
        <form
          className="flex flex-col gap-4"
          onSubmit={submit(async () => {
            await api("volumes", { name })
            closeDialog()
            toast.success(t("volumes.created", { name }))
            void refreshLists("volumes")
          })}
        >
          <DialogHeader>
            <DialogTitle>{t("volumes.new")}</DialogTitle>
            <DialogDescription>{t("volumes.newHint")}</DialogDescription>
          </DialogHeader>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor={`${field}-name`}>{t("table.name")}</FieldLabel>
              <Input
                id={`${field}-name`}
                required
                pattern="[a-zA-Z0-9][a-zA-Z0-9_.\-]+"
                title={t("volumes.namePattern")}
                placeholder="unity-projects"
                value={name}
                onChange={(event) => setName(event.target.value)}
                className="font-mono"
                autoFocus
              />
            </Field>
            <FieldError>{error}</FieldError>
          </FieldGroup>
          <Footer busy={busy} label={t("common.create")} />
        </form>
      </DialogContent>
    </Dialog>
  )
}
