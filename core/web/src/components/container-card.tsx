import * as React from "react"
import {
  RiAlertLine,
  RiPlayCircleLine,
  RiCpuLine,
  RiGamepadLine,
  RiMicLine,
  RiComputerLine,
  RiGroupLine,
  RiHourglassLine,
  RiResetLeftLine,
  RiRouteLine,
  RiShieldKeyholeLine,
  RiTimerLine,
  RiUploadLine,
  RiVolumeUpLine,
} from "@remixicon/react"
import { cn } from "cn"

import { ConceptIcon } from "@/components/concept"
import { ContainerMenu } from "@/components/container-menu"
import { ContainerPreview } from "@/components/container-preview"
import { JobLogButton } from "@/components/pending"
import { StatusBadge } from "@/components/status-badge"
import { Alert, AlertDescription } from "@/components/ui/alert"
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
import { Label } from "@/components/ui/label"
import { Progress } from "@/components/ui/progress"
import { Switch } from "@/components/ui/switch"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useFileDrop } from "@/hooks/use-file-drop"
import { attempt, containerAction, retrySetup, setInternet } from "@/lib/actions"
import { openDialog } from "@/lib/dialogs"
import { containerStatus, desktopTitle, jobProgress, jobTone, needsRetry } from "@/lib/format"
import { formatNumber, t } from "@/lib/i18n"
import { lifecycleBadges } from "@/lib/lifecycle"
import { pendingState, type Pending } from "@/lib/pending"
import { href } from "@/lib/route"
import type { ContainerItem, ContainerSpec, ImageItem } from "@/lib/types"

/** The resources and devices a container was given, as compact badges. */
export function SpecBadges({ spec, className }: { spec: ContainerSpec; className?: string }) {
  return (
    <div className={cn("flex flex-wrap gap-1.5", className)}>
      <Badge variant="outline" className="text-muted-foreground">
        <RiCpuLine />
        {spec.gpu ? t("spec.gpu") : t("dashboard.cpuOnly")}
      </Badge>
      <Badge variant="outline" className="text-muted-foreground">
        {spec.cpus ? t("spec.cpus", { count: formatNumber(spec.cpus) }) : t("spec.unlimitedCpu")}
      </Badge>
      <Badge variant="outline" className="text-muted-foreground">
        {spec.memory_mb
          ? t("spec.memory", { count: formatNumber(spec.memory_mb) })
          : t("spec.unlimitedMemory")}
      </Badge>
      {spec.audio && (
        <Badge variant="outline" className="text-muted-foreground">
          <RiVolumeUpLine />
          {t("spec.audio")}
        </Badge>
      )}
      {spec.microphone && (
        <Badge variant="outline" className="text-muted-foreground">
          <RiMicLine />
          {t("spec.microphone")}
        </Badge>
      )}
      {spec.gamepad && (
        <Badge variant="outline" className="text-muted-foreground">
          <RiGamepadLine />
          {t("spec.gamepad")}
        </Badge>
      )}
      {spec.vpn && (
        <Badge variant="outline" className="text-muted-foreground">
          <RiShieldKeyholeLine />
          {t("spec.vpn")}
        </Badge>
      )}
      {spec.proxy && (
        <Badge variant="outline" className="text-muted-foreground">
          <RiRouteLine />
          {t("spec.proxy")}
        </Badge>
      )}
      {!!spec.groups?.length && <span className="sr-only">{t("spec.groups")}:</span>}
      {spec.groups?.map((group) => (
        <Badge key={group} variant="outline" className="text-muted-foreground">
          <RiGroupLine />
          {group}
        </Badge>
      ))}
    </div>
  )
}

/**
 * When Thoric stops or deletes the container by itself, as badges whose tooltips say more, kept
 * current while the card shows how soon each comes.
 */
export function LifecycleBadges({ item, className }: { item: ContainerItem; className?: string }) {
  const [now, setNow] = React.useState(() => Math.floor(Date.now() / 1000))
  const badges = lifecycleBadges(item, now)
  const counting = badges.length > 0
  React.useEffect(() => {
    if (!counting) return
    const timer = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 30_000)
    return () => clearInterval(timer)
  }, [counting])
  if (!badges.length) return null
  return (
    <div className={cn("flex flex-wrap gap-1.5", className)}>
      {badges.map((badge) => (
        <Tooltip key={badge.kind}>
          <TooltipTrigger asChild>
            <Badge variant="outline" className="text-muted-foreground" tabIndex={0}>
              {badge.kind === "idle" ? <RiTimerLine /> : <RiHourglassLine />}
              {badge.text}
            </Badge>
          </TooltipTrigger>
          <TooltipContent>{badge.detail}</TooltipContent>
        </Tooltip>
      ))}
    </div>
  )
}

/** The more names a container's group members reach it by, beside its name. */
export function AliasBadges({ aliases }: { aliases?: string[] }) {
  if (!aliases?.length) return null
  return (
    <>
      <span className="sr-only">{t("spec.aliases")}:</span>
      {aliases.map((alias) => (
        <Badge key={alias} variant="outline" className="font-mono text-muted-foreground">
          {alias}
        </Badge>
      ))}
    </>
  )
}

/** A container in a group whose screen its members reach, since its image predates private VNC. */
export function GroupExposure({ item }: { item: ContainerItem }) {
  if (item.vnc_private !== false || !item.spec.groups?.length) return null
  return (
    <Alert>
      <RiAlertLine className="text-warning" />
      <AlertDescription>{t("containers.groupsExposed")}</AlertDescription>
    </Alert>
  )
}

/** The main action for a container's state: connect, retry its setup or start it. */
export function PrimaryAction({ item, className }: { item: ContainerItem; className?: string }) {
  const { id } = item
  const [busy, setBusy] = React.useState(false)
  const run = (action: () => Promise<unknown>) => async () => {
    setBusy(true)
    await attempt(action)
    setBusy(false)
  }
  if (item.state === "running")
    return (
      <Button asChild className={className}>
        <a href={href({ page: "desktop", id })}>
          <RiComputerLine data-icon="inline-start" />
          {t("containers.connect")}
        </a>
      </Button>
    )
  if (needsRetry(item))
    return (
      <Button
        variant="outline"
        className={className}
        disabled={busy}
        onClick={run(() => retrySetup(id))}
      >
        <RiResetLeftLine data-icon="inline-start" />
        {t("job.retry")}
      </Button>
    )
  return (
    <Button
      variant="outline"
      className={className}
      disabled={busy}
      onClick={run(() => containerAction(id, "start"))}
    >
      <RiPlayCircleLine data-icon="inline-start" />
      {t("containers.start")}
    </Button>
  )
}

export function InternetSwitch({ item, id }: { item: ContainerItem; id?: string }) {
  const [busy, setBusy] = React.useState(false)
  return (
    <Switch
      id={id}
      checked={item.spec.internet}
      disabled={busy}
      aria-label={t("containers.internetOf", { name: item.spec.name })}
      onCheckedChange={async (enabled) => {
        setBusy(true)
        await attempt(() => setInternet(item.id, enabled))
        setBusy(false)
      }}
    />
  )
}

/**
 * A container with its live preview and controls. `aliases` shows its aliases beside its name, as
 * a group's page does, and `actions` adds buttons before its menu.
 */
export function ContainerCard({
  item,
  images,
  aliases = false,
  actions,
}: {
  item: ContainerItem
  images: ImageItem[]
  aliases?: boolean
  actions?: React.ReactNode
}) {
  const { id, spec } = item
  const running = item.state === "running"
  const status = containerStatus(item)
  const drop = useFileDrop((files) => openDialog({ kind: "upload", id, files }))
  const switchId = React.useId()

  return (
    <Card
      {...drop.props}
      className={cn(
        "gap-4 pt-3 transition-colors hover:ring-foreground/20",
        drop.over && "ring-2 ring-primary"
      )}
    >
      <CardContent className="px-3">
        <ContainerPreview item={item} />
      </CardContent>
      <CardHeader>
        <CardTitle className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-base">
          <ConceptIcon concept="container" className="size-4 shrink-0" />
          <button
            type="button"
            className="max-w-full min-w-0 truncate text-left hover:underline focus-visible:underline focus-visible:outline-none"
            onClick={() => openDialog({ kind: "details", id })}
          >
            {spec.name}
          </button>
          {aliases && <AliasBadges aliases={spec.aliases} />}
        </CardTitle>
        <CardDescription className="truncate">
          {desktopTitle(spec, item.desktop?.title, images)}
        </CardDescription>
        <CardAction>
          <StatusBadge state={status.key} tone={status.tone} />
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-col gap-1.5">
          <SpecBadges spec={spec} />
          <LifecycleBadges item={item} />
        </div>
        <GroupExposure item={item} />
        <div className="flex items-center justify-between">
          <Label htmlFor={switchId} className="text-muted-foreground">
            {t("spec.internet")}
          </Label>
          <InternetSwitch item={item} id={switchId} />
        </div>
        {item.last_error && (
          <Alert variant="destructive">
            <AlertDescription className="line-clamp-3">{item.last_error}</AlertDescription>
          </Alert>
        )}
      </CardContent>
      <CardFooter className="mt-auto gap-2">
        <PrimaryAction item={item} className="flex-1" />
        <Tooltip>
          <TooltipTrigger asChild>
            <span tabIndex={running ? -1 : 0}>
              <Button
                variant="outline"
                size="icon"
                disabled={!running}
                onClick={() => openDialog({ kind: "upload", id })}
                aria-label={t("uploads.to", { name: spec.name })}
              >
                <RiUploadLine />
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>{running ? t("uploads.title") : t("uploads.startFirst")}</TooltipContent>
        </Tooltip>
        {actions}
        <ContainerMenu item={item} />
      </CardFooter>
    </Card>
  )
}

/**
 * A container its creation job is still making. It has nothing to control yet, so the card shows
 * how far the job is and opens the job's panel instead.
 */
export function PendingContainerCard({ pending }: { pending: Pending }) {
  const { job, name } = pending
  const progress = job.status === "running" ? jobProgress(job.progress) : null
  const queued = job.status === "queued"
  return (
    <Card className="gap-4 pt-3">
      <CardContent className="px-3">
        <div
          role="status"
          className="flex aspect-video flex-col items-center justify-center gap-2 rounded-md border bg-muted/40 px-6 text-center text-xs text-muted-foreground"
        >
          <RiComputerLine className="size-6 opacity-60" aria-hidden />
          {progress?.text ?? (queued ? t("jobs.log.queued") : t("containers.creating"))}
          {progress?.max !== undefined && (
            <Progress
              value={(progress.value! * 100) / progress.max}
              aria-label={progress.title}
              className="max-w-48 bg-background"
            />
          )}
        </div>
      </CardContent>
      <CardHeader>
        <CardTitle className="flex min-w-0 items-center gap-2 text-base">
          <ConceptIcon concept="container" className="size-4 shrink-0" />
          <button
            type="button"
            className="truncate text-left hover:underline focus-visible:underline focus-visible:outline-none"
            onClick={() => openDialog({ kind: "job", id: job.id })}
          >
            {name}
          </button>
        </CardTitle>
        <CardDescription className="line-clamp-2">{t("containers.creatingHint")}</CardDescription>
        <CardAction>
          <StatusBadge state={pendingState(job)} tone={jobTone(job.status)} />
        </CardAction>
      </CardHeader>
      <CardFooter className="mt-auto">
        <JobLogButton id={job.id} size="default" className="flex-1" />
      </CardFooter>
    </Card>
  )
}
