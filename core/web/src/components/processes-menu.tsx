import * as React from "react"
import { RiCpuLine } from "@remixicon/react"

import { ConceptIcon } from "@/components/concept"
import { buttonLabels } from "@/components/jobs-menu"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { openDialog } from "@/lib/dialogs"
import { formatNumber, t } from "@/lib/i18n"
import { runningContainers } from "@/lib/processes"
import { useContainers } from "@/lib/queries"
import { fileSize } from "@/lib/uploads"
import { useUsage } from "@/lib/usage"

/** The running containers with what they use now; it reads the usage only while the menu is open. */
function RunningContainers() {
  const containers = useContainers(false)
  const { history } = useUsage()
  const running = React.useMemo(
    () => runningContainers(containers.data ?? [], history?.containers ?? [], history?.latest ?? 0),
    [containers.data, history]
  )
  if (!running.length)
    return (
      <p className="px-3 py-2 text-sm text-muted-foreground">
        {containers.isPending ? t("common.loading") : t("processes.noneRunning")}
      </p>
    )
  return running.map((item) => (
    <DropdownMenuItem key={item.id} onSelect={() => openDialog({ kind: "processes", id: item.id })}>
      <ConceptIcon concept="container" />
      <span className="min-w-0 flex-1 truncate">{item.name}</span>
      <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
        {item.cpu === null || item.memory === null
          ? "…"
          : t("processes.usage", {
              cpu: t("usage.unit.cpu", {
                value: formatNumber(item.cpu, { maximumFractionDigits: item.cpu < 10 ? 2 : 1 }),
              }),
              memory: fileSize(item.memory),
            })}
      </span>
    </DropdownMenuItem>
  ))
}

/**
 * The processes beside the jobs at the top right of every page: the running containers with what
 * each uses, each opening its process list. With `container`, as in the desktop view, the button
 * opens that container's list at once.
 */
export function ProcessesMenu({
  container,
  labelFrom = "sm",
}: {
  container?: string
  labelFrom?: keyof typeof buttonLabels
}) {
  const label = t("nav.processes")
  const button = (props: React.ComponentProps<typeof Button>) => (
    <Button variant="ghost" size="sm" aria-label={label} {...props}>
      <RiCpuLine />
      <span className={buttonLabels[labelFrom].label}>{label}</span>
    </Button>
  )
  if (container)
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          {button({ onClick: () => openDialog({ kind: "processes", id: container }) })}
        </TooltipTrigger>
        <TooltipContent className={buttonLabels[labelFrom].tooltip}>{label}</TooltipContent>
      </Tooltip>
    )
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>{button({})}</DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent className={buttonLabels[labelFrom].tooltip}>{label}</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end" className="w-80 max-w-[calc(100vw-2rem)]">
        <DropdownMenuLabel className="font-medium text-foreground">
          {t("processes.running")}
        </DropdownMenuLabel>
        <RunningContainers />
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
