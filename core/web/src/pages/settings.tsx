import * as React from "react"
import {
  RiArrowRightSLine,
  RiArrowUpCircleLine,
  RiExternalLinkLine,
  RiRefreshLine,
} from "@remixicon/react"
import { cn } from "cn"
import { toast } from "sonner"

import { BackupCard } from "@/components/backup-card"
import { ConfigurationEditor } from "@/components/configuration-editor"
import { NotificationsCard } from "@/components/notifications-card"
import { PageBody, SectionHeading } from "@/components/page"
import { StatusBadge } from "@/components/status-badge"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { api } from "@/lib/api"
import { openDialog } from "@/lib/dialogs"
import { shortDate } from "@/lib/format"
import { t } from "@/lib/i18n"
import { queryClient, useUpdate } from "@/lib/queries"
import { href } from "@/lib/route"
import { offered, releaseState } from "@/lib/settings"
import type { UpdateStatus } from "@/lib/types"
import { buildName } from "@/lib/update"

function ReleaseBadge({ state }: { state: ReturnType<typeof releaseState> }) {
  const label = t(`settings.state.${state}`)
  if (state === "available")
    return (
      <Badge variant="outline" className="px-1.5 text-muted-foreground">
        <RiArrowUpCircleLine className="text-info" />
        {label}
      </Badge>
    )
  const tone = {
    current: "success",
    updating: "info",
    failed: "danger",
    off: "muted",
    unchecked: "muted",
  } as const
  return <StatusBadge state={state} tone={tone[state]} label={label} />
}

function Fact({
  label,
  className,
  children,
}: {
  label: string
  className?: string
  children: React.ReactNode
}) {
  return (
    <div className={cn("flex flex-col gap-0.5", className)}>
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="font-medium">{children}</dd>
    </div>
  )
}

/** What a check found, as its toast says it. */
function announceCheck(status: UpdateStatus) {
  const edge = status.edge
  if (edge) {
    if (edge.available) toast.info(t("update.edgeAvailable"))
    else if (edge.error) toast.error(t("settings.edgeCheckFailed", { error: edge.error }))
    else toast.success(t("settings.upToDateEdge", { image: edge.image }))
  } else if (status.available && status.newest)
    toast.info(t("update.available", { version: status.newest }))
  else if (status.error) toast.error(t("settings.checkFailed", { error: status.error }))
  else toast.success(t("settings.upToDate", { version: status.current }))
}

/**
 * This release against the newest one, or for an installation that follows `edge` this build
 * against the newest one, with the manual check and the update.
 */
function VersionCard() {
  const update = useUpdate().data
  const [checking, setChecking] = React.useState(false)
  if (!update) return <Skeleton className="h-48 rounded-2xl" />
  const state = releaseState(update)
  const job = update.job_id
  const edge = update.edge
  const notes = edge ? edge.available && edge.changes_url : update.available && update.release_url

  const check = async () => {
    setChecking(true)
    try {
      const status = await api<UpdateStatus>("update/check", {})
      queryClient.setQueryData(["update"], status)
      announceCheck(status)
    } catch (error) {
      toast.error((error as Error).message)
    } finally {
      setChecking(false)
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("settings.release", { version: buildName(update) })}</CardTitle>
        <CardDescription>
          {state === "off"
            ? t("settings.checksOff")
            : edge
              ? t("settings.edgeHint")
              : t("settings.versionHint")}
        </CardDescription>
        <CardAction>
          <ReleaseBadge state={state} />
        </CardAction>
      </CardHeader>
      {update.enabled && (
        <CardContent className="flex flex-col gap-4">
          <dl className="grid grid-cols-2 gap-4 sm:max-w-md">
            <Fact label={t("settings.newest")}>
              {update.newest ? (
                <span className="font-mono">{update.newest}</span>
              ) : (
                <span className="text-muted-foreground">{t("common.unknown")}</span>
              )}
            </Fact>
            <Fact label={t("settings.checkedAt")}>
              {update.checked_at ? (
                shortDate(update.checked_at)
              ) : (
                <span className="text-muted-foreground">{t("settings.never")}</span>
              )}
            </Fact>
            {edge && (
              <Fact label={t("settings.edgeImage")} className="col-span-2">
                <span className="font-mono break-all">{edge.image}</span>
              </Fact>
            )}
          </dl>
          {update.error && (
            <p className="text-sm text-destructive">
              {t("settings.checkFailed", { error: update.error })}
            </p>
          )}
          {edge?.error && (
            <p className="text-sm text-destructive">
              {t("settings.edgeCheckFailed", { error: edge.error })}
            </p>
          )}
          {state === "updating" && (
            <p className="text-sm text-muted-foreground">
              {offered(update) ? t("update.runningDetail") : t("update.finishing")}
            </p>
          )}
          {state === "available" && !update.installable && (
            <p className="text-sm text-muted-foreground">
              {edge
                ? t("update.edgeManual", { image: edge.image })
                : t("update.manual", { version: update.current })}
            </p>
          )}
        </CardContent>
      )}
      {update.enabled && (
        <CardFooter className="flex flex-wrap gap-2">
          {job ? (
            <Button
              variant="outline"
              size="sm"
              onClick={() => openDialog({ kind: "job", id: job })}
            >
              <Spinner data-icon="inline-start" />
              {t("update.showLog")}
            </Button>
          ) : (
            <Button variant="outline" size="sm" disabled={checking} onClick={() => void check()}>
              {checking ? <Spinner data-icon="inline-start" /> : <RiRefreshLine />}
              {t("settings.check")}
            </Button>
          )}
          {state === "available" && update.installable && (edge || update.newest) && (
            <Button size="sm" onClick={() => openDialog({ kind: "update" })}>
              <RiArrowUpCircleLine />
              {edge
                ? t("settings.updateEdge")
                : t("settings.updateTo", { version: update.newest ?? "" })}
            </Button>
          )}
          {notes && (
            <Button variant="ghost" size="sm" asChild>
              <a href={notes} target="_blank" rel="noreferrer">
                {t("update.notes")}
                <RiExternalLinkLine data-icon="inline-end" />
              </a>
            </Button>
          )}
        </CardFooter>
      )}
      <CardFooter className="flex-wrap gap-x-1.5 gap-y-1 border-t text-xs text-muted-foreground">
        {t("settings.notice")}
        <a
          href={href({ page: "dependencies" })}
          className="inline-flex items-center gap-0.5 rounded-sm font-medium text-foreground underline-offset-4 outline-none hover:underline focus-visible:ring-3 focus-visible:ring-ring/50"
        >
          {t("settings.noticeLink")}
          <RiArrowRightSLine className="size-3.5" />
        </a>
      </CardFooter>
    </Card>
  )
}

export function SettingsPage() {
  return (
    <PageBody>
      <SectionHeading title={t("settings.version")} />
      <div className="px-4 lg:px-6">
        <VersionCard />
      </div>
      <SectionHeading title={t("backup.title")} description={t("backup.description")} />
      <div className="px-4 lg:px-6">
        <BackupCard />
      </div>
      <SectionHeading title={t("notify.title")} />
      <div className="px-4 lg:px-6">
        <NotificationsCard />
      </div>
      <SectionHeading
        title={t("settings.configuration")}
        description={t("settings.configurationHint")}
      />
      <ConfigurationEditor />
    </PageBody>
  )
}
