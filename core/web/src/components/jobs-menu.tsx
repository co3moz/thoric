import * as React from "react"
import { RiArrowRightSLine, RiListCheck3 } from "@remixicon/react"

import { StatusIcon } from "@/components/status-badge"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { openDialog } from "@/lib/dialogs"
import {
  currentStage,
  jobActive,
  jobName,
  jobProgress,
  jobTarget,
  jobTone,
  recentJobs,
  shortDate,
  stateLabel,
} from "@/lib/format"
import { plural, t } from "@/lib/i18n"
import { setShowScripts, useShowScripts } from "@/lib/job-history"
import { useContainers, useImages, useJobList, useJobs } from "@/lib/queries"
import { href } from "@/lib/route"
import type { Job } from "@/lib/types"

const LIMIT = 6

// The button's word shows from this width on; below it the tooltip names the button instead.
export const buttonLabels = {
  sm: { label: "hidden sm:inline", tooltip: "sm:hidden" },
  xl: { label: "hidden xl:inline", tooltip: "xl:hidden" },
}

function JobItem({ job, target }: { job: Job; target: string }) {
  const percent = job.status === "running" ? jobProgress(job.progress)?.percent : undefined
  const state = currentStage(job) ?? stateLabel(job.status)
  return (
    <DropdownMenuItem
      className="items-start"
      onSelect={() => openDialog({ kind: "job", id: job.id })}
    >
      <span className="mt-0.5 flex">
        <StatusIcon tone={jobTone(job.status)} />
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex items-baseline gap-2">
          <span className="min-w-0 flex-1 truncate font-medium">{jobName(job.kind)}</span>
          <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
            {percent !== undefined
              ? `${percent}%`
              : jobActive(job)
                ? null
                : shortDate(job.created_at)}
          </span>
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {target ? `${target} · ${state}` : state}
        </span>
      </span>
    </DropdownMenuItem>
  )
}

/** The number of jobs in progress, round for one digit; nothing while none run. */
function CountBadge({ count }: { count: number }) {
  if (!count) return null
  return (
    <Badge aria-hidden className="h-5 min-w-5 px-1 tabular-nums">
      {count}
    </Badge>
  )
}

/** The jobs in progress beside the Jobs item of the sidebar, in words for screen readers. */
export function ActiveJobsBadge() {
  const count = useJobs().data?.filter(jobActive).length ?? 0
  if (!count) return null
  return (
    <span className="ml-auto flex">
      <CountBadge count={count} />
      <span className="sr-only">
        {plural(count, { one: "jobs.inProgress.one", other: "jobs.inProgress.other" })}
      </span>
    </span>
  )
}

/**
 * The jobs at the top right of every page: the number in progress on the button, the latest jobs
 * in its menu with their logs a click away, with or without the scripts among them, and the
 * whole list on the jobs page.
 */
export function JobsMenu({ labelFrom = "sm" }: { labelFrom?: keyof typeof buttonLabels }) {
  const scripts = useShowScripts()
  const jobs = useJobList(scripts)
  const containers = useContainers(false).data
  const images = useImages(false).data
  // The button counts every job in progress, as the sidebar does, whatever the menu lists.
  const active = useJobs().data?.filter(jobActive).length ?? 0
  const recent = React.useMemo(() => {
    const names = new Map(containers?.map((container) => [container.id, container.spec.name]))
    const imageNames = new Map(images?.map((image) => [image.id, image.name]))
    return recentJobs(jobs.data ?? [], LIMIT).map((job) => ({
      job,
      target: jobTarget(job, names, imageNames),
    }))
  }, [jobs.data, containers, images])
  const progress = active
    ? plural(active, { one: "jobs.inProgress.one", other: "jobs.inProgress.other" })
    : undefined

  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="sm"
              aria-label={progress ? `${t("nav.jobs")}, ${progress}` : t("nav.jobs")}
            >
              <RiListCheck3 />
              <span className={buttonLabels[labelFrom].label}>{t("nav.jobs")}</span>
              <CountBadge count={active} />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent className={buttonLabels[labelFrom].tooltip}>
          {progress ?? t("nav.jobs")}
        </TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end" className="w-80 max-w-[calc(100vw-2rem)]">
        <DropdownMenuLabel className="flex items-center justify-between gap-2">
          <span className="font-medium text-foreground">{t("nav.jobs")}</span>
          {progress}
        </DropdownMenuLabel>
        {recent.length ? (
          recent.map(({ job, target }) => <JobItem key={job.id} job={job} target={target} />)
        ) : (
          <p className="px-3 py-2 text-sm text-muted-foreground">
            {jobs.isPending ? t("jobs.loading") : t("jobs.none")}
          </p>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuCheckboxItem
          checked={scripts}
          onCheckedChange={setShowScripts}
          // The menu stays open, so the list is seen to change.
          onSelect={(event) => event.preventDefault()}
        >
          {t("jobs.showScripts")}
        </DropdownMenuCheckboxItem>
        <DropdownMenuItem asChild>
          <a href={href({ page: "jobs" })}>
            <RiListCheck3 />
            {t("jobs.view.all")}
            <RiArrowRightSLine className="ml-auto text-muted-foreground" />
          </a>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
