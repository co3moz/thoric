import * as React from "react"
import { RiArrowUpCircleLine, RiExternalLinkLine } from "@remixicon/react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Spinner } from "@/components/ui/spinner"
import { openDialog } from "@/lib/dialogs"
import { t } from "@/lib/i18n"
import { useUpdate } from "@/lib/queries"
import { offered } from "@/lib/settings"
import type { UpdateStatus } from "@/lib/types"
import { build, followUpdate, updatedFrom } from "@/lib/update"

/** What the card says about the installation: the release, or the edge build, it runs. */
function runs(update: UpdateStatus) {
  if (update.edge && !update.installable)
    return t("update.edgeManual", { image: update.edge.image })
  if (update.edge && update.commit)
    return t("update.edgeCurrent", { commit: update.commit.slice(0, 7) })
  return update.installable
    ? t("update.current", { version: update.current })
    : t("update.manual", { version: update.current })
}

/**
 * A newer release in the sidebar, or a newer build of `edge` for an installation that follows it:
 * what it is, its notes and, where the WSL package takes the request, the update. While one runs,
 * the page follows it to the new manager.
 */
export function UpdateNotice() {
  const update = useUpdate().data
  const current = update?.current
  const commit = update?.commit ?? null
  const from = current && build({ current, commit })
  const job = update?.job_id

  React.useEffect(() => {
    if (!current || !from) return
    const before = updatedFrom()
    if (!before || before === from) return
    toast.success(
      commit
        ? t("update.edgeDone", { commit: commit.slice(0, 7) })
        : t("update.done", { version: current })
    )
  }, [from, current, commit])
  React.useEffect(() => {
    if (job && from) followUpdate(from)
  }, [job, from])

  if (!update?.enabled) return null
  if (job)
    return (
      <Card size="sm" className="shadow-none">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-sm">
            <Spinner className="size-4" />
            {t("update.running")}
          </CardTitle>
          <CardDescription className="text-xs">
            {/* Once the new manager answers, it follows the rest, such as the desktop images. */}
            {offered(update) ? t("update.runningDetail") : t("update.finishing")}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button
            size="sm"
            variant="outline"
            className="w-full"
            onClick={() => openDialog({ kind: "job", id: job })}
          >
            {t("update.showLog")}
          </Button>
        </CardContent>
      </Card>
    )
  if (!offered(update) || (!update.edge && !update.newest)) return null
  const notes = update.edge ? update.edge.changes_url : update.release_url
  return (
    <Card size="sm" className="shadow-none">
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-sm">
          <RiArrowUpCircleLine className="size-4 shrink-0 text-info" />
          {update.edge
            ? t("update.edgeAvailable")
            : t("update.available", { version: update.newest ?? "" })}
        </CardTitle>
        <CardDescription className="text-xs">{runs(update)}</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        {update.installable && (
          <Button size="sm" onClick={() => openDialog({ kind: "update" })}>
            {t("update.install")}
          </Button>
        )}
        {notes && (
          <Button size="sm" variant="outline" asChild>
            <a href={notes} target="_blank" rel="noreferrer">
              {t("update.notes")}
              <RiExternalLinkLine data-icon="inline-end" />
            </a>
          </Button>
        )}
      </CardContent>
    </Card>
  )
}
