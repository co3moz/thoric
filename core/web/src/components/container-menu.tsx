import {
  RiCameraLine,
  RiCpuLine,
  RiPlayCircleLine,
  RiStopCircleLine,
  RiMore2Line,
  RiFileTextLine,
  RiInformationLine,
  RiResetLeftLine,
  RiRestartLine,
  RiSettings3Line,
  RiStethoscopeLine,
  RiDeleteBinLine,
  RiDownloadLine,
  RiEraserLine,
  RiFileCopyLine,
  RiStackLine,
  RiUploadLine,
} from "@remixicon/react"

import { ConceptIcon } from "@/components/concept"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { attempt, containerAction, retrySetup, showLog } from "@/lib/actions"
import { openDialog } from "@/lib/dialogs"
import { needsRetry } from "@/lib/format"
import { t } from "@/lib/i18n"
import type { ContainerItem } from "@/lib/types"

/** Every action on one container, as the old details dialog offered them. */
export function ContainerMenu({ item }: { item: ContainerItem }) {
  const { id, spec } = item
  const running = item.state === "running"
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          className="flex size-8 text-muted-foreground data-[state=open]:bg-muted"
          size="icon"
        >
          <RiMore2Line />
          <span className="sr-only">{t("containers.actionsOf", { name: spec.name })}</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-48">
        <DropdownMenuItem tone="info" onSelect={() => openDialog({ kind: "details", id })}>
          <RiInformationLine />
          {t("containers.details")}
        </DropdownMenuItem>
        {needsRetry(item) && (
          <DropdownMenuItem tone="warning" onSelect={() => attempt(() => retrySetup(id))}>
            <RiResetLeftLine />
            {t("job.retry")}
          </DropdownMenuItem>
        )}
        {!running && (
          <DropdownMenuItem
            tone="success"
            onSelect={() => attempt(() => containerAction(id, "start"))}
          >
            <RiPlayCircleLine />
            {t("containers.start")}
          </DropdownMenuItem>
        )}
        {running && (
          <DropdownMenuItem
            tone="warning"
            onSelect={() => attempt(() => containerAction(id, "stop"))}
          >
            <RiStopCircleLine />
            {t("containers.stop")}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem
          tone="success"
          onSelect={() => attempt(() => containerAction(id, "restart"))}
        >
          <RiRestartLine />
          {t("containers.restart")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          tone="transfer"
          disabled={!running}
          onSelect={() => openDialog({ kind: "upload", id })}
        >
          <RiUploadLine />
          {t("uploads.title")}
        </DropdownMenuItem>
        <DropdownMenuItem
          tone="transfer"
          disabled={!running}
          onSelect={() => openDialog({ kind: "download", id })}
        >
          <RiDownloadLine />
          {t("downloads.title")}
        </DropdownMenuItem>
        <DropdownMenuItem tone="edit" onSelect={() => openDialog({ kind: "settings", id })}>
          <RiSettings3Line />
          {t("containers.settings")}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => openDialog({ kind: "snapshot", id })}>
          <ConceptIcon concept="image" icon={RiCameraLine} />
          {t("containers.snapshot")}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => openDialog({ kind: "duplicate", id })}>
          <ConceptIcon concept="container" icon={RiFileCopyLine} />
          {t("containers.duplicate")}
        </DropdownMenuItem>
        {item.scale ? (
          <DropdownMenuItem
            onSelect={() => openDialog({ kind: "scale-instance", name: item.scale!.set })}
          >
            <ConceptIcon concept="container" icon={RiStackLine} />
            {t("scale.addInstance")}
          </DropdownMenuItem>
        ) : (
          <DropdownMenuItem onSelect={() => openDialog({ kind: "make-scalable", id })}>
            <ConceptIcon concept="container" icon={RiStackLine} />
            {t("scale.make")}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem
          tone="info"
          onSelect={() =>
            attempt(() =>
              showLog(t("containers.logsOf", { name: spec.name }), `containers/${id}/logs`)
            )
          }
        >
          <RiFileTextLine />
          {t("containers.logs")}
        </DropdownMenuItem>
        {running && (
          <DropdownMenuItem tone="info" onSelect={() => openDialog({ kind: "processes", id })}>
            <RiCpuLine />
            {t("nav.processes")}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem
          tone="info"
          onSelect={() =>
            attempt(() =>
              showLog(
                t("containers.diagnosticsOf", { name: spec.name }),
                `containers/${id}/diagnostics`
              )
            )
          }
        >
          <RiStethoscopeLine />
          {t("containers.diagnostics")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onSelect={() => openDialog({ kind: "reset", id })}>
          <RiEraserLine />
          {t("containers.reset")}
        </DropdownMenuItem>
        <DropdownMenuItem variant="destructive" onSelect={() => openDialog({ kind: "delete", id })}>
          <RiDeleteBinLine />
          {t("job.delete")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
