import { RiAlertLine } from "@remixicon/react"
import { toast } from "sonner"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
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
  restorePlan,
  restoreUpload,
  type BackupConflict,
  type BackupManifest,
  type BackupReview,
} from "@/lib/backup"
import { closeDialog, openDialog } from "@/lib/dialogs"
import { formatList, t } from "@/lib/i18n"
import { refreshLists } from "@/lib/queries"
import { shortDate } from "@/lib/format"
import type { DialogProps } from "./side-drawer"

/**
 * Uploads a backup whose restore was confirmed. The upload goes on while another page shows; once
 * the whole backup has arrived its job takes over, and the job panel opens.
 */
async function startRestore(file: File, limit: number) {
  const upload = restoreUpload()
  const job = await upload.start(file, {}, limit)
  if (!job) {
    if (upload.status === "failed") toast.error(upload.error)
    return
  }
  toast.success(t("backup.restoring"), { description: t("backup.restoringDetail") })
  void refreshLists("jobs")
  openDialog({ kind: "job", id: job })
}

const conflictKeys = {
  container: "backup.conflict.container",
  image: "backup.conflict.image",
  endpoint: "backup.conflict.endpoint",
  volume: "backup.conflict.volume",
  "scale-set": "backup.conflict.scaleSet",
} as const

function conflictLabel(conflict: BackupConflict) {
  return t(conflictKeys[conflict.kind] ?? "backup.conflict.container", { name: conflict.name })
}

function Names({ title, names, mono }: { title: string; names: string[]; mono?: boolean }) {
  if (!names.length) return null
  return (
    <div className="flex flex-col gap-1">
      <dt className="text-xs text-muted-foreground">{title}</dt>
      <dd className={mono ? "font-mono text-xs break-all" : "text-sm break-words"}>
        {formatList(names)}
      </dd>
    </div>
  )
}

/** Names what a restore creates and what is in its way, and starts it once confirmed. */
export function RestoreDialog({
  file,
  manifest,
  review,
  limit,
  ...props
}: DialogProps & {
  file: File
  manifest: BackupManifest
  review: BackupReview
  limit: number
}) {
  const plan = restorePlan(manifest)
  const blocked = review.conflicts.length > 0
  const empty =
    !plan.containers.length &&
    !plan.withoutFiles.length &&
    !plan.images.length &&
    !plan.recordsOnly.length &&
    !plan.volumes.length &&
    !plan.endpoints.length &&
    !plan.scaleSets.length
  return (
    <AlertDialog {...props}>
      <AlertDialogContent className="sm:max-w-lg">
        <form
          className="contents"
          onSubmit={(event) => {
            event.preventDefault()
            if (blocked) return
            closeDialog()
            void startRestore(file, limit)
          }}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t("backup.confirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("backup.confirmFrom", {
                version: manifest.version,
                date: shortDate(manifest.created_at),
              })}{" "}
              {t("backup.stopped")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <div className="flex min-h-0 flex-col gap-3 overflow-y-auto">
            {empty ? (
              <p className="text-sm text-muted-foreground">{t("backup.nothing")}</p>
            ) : (
              <dl className="flex flex-col gap-3 rounded-lg border p-3">
                <Names title={t("backup.list.containers")} names={plan.containers} />
                <Names title={t("backup.list.withoutFiles")} names={plan.withoutFiles} />
                <Names title={t("backup.list.images")} names={plan.images} />
                <Names title={t("backup.list.recordsOnly")} names={plan.recordsOnly} />
                <Names title={t("backup.list.volumes")} names={plan.volumes} mono />
                <Names title={t("backup.list.endpoints")} names={plan.endpoints} mono />
                <Names title={t("backup.list.scaleSets")} names={plan.scaleSets} mono />
              </dl>
            )}
            {plan.withoutFiles.length > 0 && (
              <p className="text-xs text-muted-foreground">{t("backup.withoutFilesHint")}</p>
            )}
            {plan.secretsLeftOut.length > 0 && (
              <p className="text-xs text-muted-foreground">
                {t("backup.secretsLeftOut", { names: formatList(plan.secretsLeftOut) })}
              </p>
            )}
            {review.missing_volumes.length > 0 && (
              <Alert>
                <RiAlertLine className="text-warning" />
                <AlertDescription>
                  {t("backup.missingVolumes", { names: formatList(review.missing_volumes) })}
                </AlertDescription>
              </Alert>
            )}
            {blocked && (
              <Alert variant="destructive">
                <RiAlertLine />
                <AlertTitle>{t("backup.conflicts")}</AlertTitle>
                <AlertDescription>
                  <ul className="list-disc pl-4">
                    {review.conflicts.map((conflict) => (
                      <li key={`${conflict.kind}:${conflict.name}`} className="break-words">
                        {conflictLabel(conflict)}
                      </li>
                    ))}
                  </ul>
                  <p>{t("backup.conflictsHint")}</p>
                </AlertDescription>
              </Alert>
            )}
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel type="button">{t("common.cancel")}</AlertDialogCancel>
            <Button type="submit" disabled={blocked}>
              {t("backup.restoreAction")}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
