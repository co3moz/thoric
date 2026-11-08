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
import { FieldError } from "@/components/ui/field"
import { Spinner } from "@/components/ui/spinner"
import { api } from "@/lib/api"
import { openDialog } from "@/lib/dialogs"
import { t } from "@/lib/i18n"
import { queryClient, useUpdate } from "@/lib/queries"
import { offered } from "@/lib/settings"
import type { JobStarted } from "@/lib/types"
import { build, followUpdate } from "@/lib/update"
import { useSubmit } from "./container-dialogs"
import type { DialogProps } from "./side-drawer"

/**
 * Asks before the WSL package runs `thoric update`, which replaces the manager with the newest
 * release, or with the newest build of `edge` for an installation that follows it.
 */
export function UpdateDialog(props: DialogProps) {
  const update = useUpdate().data
  const version = update?.newest ?? ""
  const edge = update?.edge
  const { busy, error, submit } = useSubmit()
  return (
    <AlertDialog {...props}>
      <AlertDialogContent>
        <form
          className="contents"
          onSubmit={submit(async () => {
            const { job_id } = await api<JobStarted>("update", {})
            if (update) followUpdate(build(update))
            void queryClient.invalidateQueries({ queryKey: ["update"] })
            // The job's log follows the update until the new manager takes over.
            openDialog({ kind: "job", id: job_id })
          })}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>
              {edge ? t("update.edgeConfirmTitle") : t("update.confirmTitle", { version })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {edge
                ? t("update.edgeConfirmText", { image: edge.image })
                : t("update.confirmText", { version })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <FieldError>{error}</FieldError>
          <AlertDialogFooter>
            <AlertDialogCancel type="button">{t("common.cancel")}</AlertDialogCancel>
            <Button type="submit" disabled={busy || !update || !offered(update)}>
              {busy && <Spinner data-icon="inline-start" />}
              {t("update.install")}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}
