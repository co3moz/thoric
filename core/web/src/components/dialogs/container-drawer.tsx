import * as React from "react"
import {
  RiCameraLine,
  RiStopCircleLine,
  RiFileTextLine,
  RiResetLeftLine,
  RiRestartLine,
  RiSettings3Line,
  RiStethoscopeLine,
  RiDeleteBinLine,
  RiDownloadLine,
  RiEraserLine,
  RiUploadLine,
} from "@remixicon/react"

import { ConceptIcon } from "@/components/concept"
import { ContainerActivity } from "@/components/container-activity"
import {
  GroupExposure,
  LifecycleBadges,
  PrimaryAction,
  SpecBadges,
} from "@/components/container-card"
import { ContainerPreview } from "@/components/container-preview"
import { ErrorBoundary } from "@/components/error-boundary"
import { StatusBadge } from "@/components/status-badge"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  DrawerClose,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer"
import { Separator } from "@/components/ui/separator"
import { Skeleton } from "@/components/ui/skeleton"
import { attempt, containerAction, retrySetup, showLog } from "@/lib/actions"
import { openDialog } from "@/lib/dialogs"
import { containerStatus, date, desktopTitle } from "@/lib/format"
import { groupNames, groupPeers } from "@/lib/groups"
import { formatList, formatNumber, rich, t } from "@/lib/i18n"
import { useContainers, useImages } from "@/lib/queries"
import { setupGroups, setupLabel } from "@/lib/compose"
import type { ContainerSetup } from "@/lib/types"
import { SideDrawer, type DialogProps } from "./side-drawer"

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="break-words">{children}</dd>
    </div>
  )
}

// The charts load with the charting library, after the drawer has opened.
const ContainerUsage = React.lazy(() =>
  import("@/components/usage-charts").then((module) => ({ default: module.ContainerUsage }))
)

const yes = (value: boolean) => (value ? t("common.on") : t("common.off"))

const setupTone = {
  pending: "warning",
  running: "info",
  completed: "success",
  failed: "danger",
} as const satisfies Record<ContainerSetup["status"], string>

/** dashboard-01's row viewer for one container: preview, configuration and every action. */
export function ContainerDrawer({ id, ...props }: DialogProps & { id: string }) {
  const containers = useContainers().data ?? []
  const item = containers.find((entry) => entry.id === id)
  const images = useImages().data ?? []
  if (!item)
    return (
      <SideDrawer {...props}>
        <DrawerHeader>
          <DrawerTitle>{t("containers.container")}</DrawerTitle>
          <DrawerDescription>{t("containers.gone")}</DrawerDescription>
        </DrawerHeader>
      </SideDrawer>
    )
  const { spec, setup } = item
  const running = item.state === "running"
  const status = containerStatus(item)
  // A setup still waiting after an error did not get to run, which reads as a failure.
  const setupStatus = setup?.status === "pending" && item.last_error ? "failed" : setup?.status

  return (
    <SideDrawer {...props}>
      <DrawerHeader className="gap-1">
        <DrawerTitle className="flex items-center justify-center gap-2 md:justify-start">
          <ConceptIcon concept="container" className="size-4 shrink-0" />
          {spec.name}
        </DrawerTitle>
        <DrawerDescription>{desktopTitle(spec, item.desktop?.title, images)}</DrawerDescription>
      </DrawerHeader>
      <div className="flex flex-col gap-4 overflow-y-auto px-4 text-sm">
        <ContainerPreview item={item} />
        <div className="flex flex-wrap items-center gap-2">
          <StatusBadge state={status.key} tone={status.tone} />
          {item.status && <span className="text-xs text-muted-foreground">{item.status}</span>}
        </div>
        {item.last_error && (
          <Alert variant="destructive">
            <AlertTitle>{t("containers.setupUnfinished")}</AlertTitle>
            <AlertDescription>{item.last_error}</AlertDescription>
          </Alert>
        )}
        <div className="grid grid-cols-2 gap-2">
          <Button variant="outline" onClick={() => openDialog({ kind: "settings", id })}>
            <RiSettings3Line data-icon="inline-start" />
            {t("containers.settings")}
          </Button>
          <Button variant="outline" onClick={() => openDialog({ kind: "snapshot", id })}>
            <RiCameraLine data-icon="inline-start" />
            {t("containers.snapshot")}
          </Button>
          <Button
            variant="outline"
            disabled={!running}
            onClick={() => openDialog({ kind: "upload", id })}
          >
            <RiUploadLine data-icon="inline-start" />
            {t("uploads.title")}
          </Button>
          <Button
            variant="outline"
            disabled={!running}
            onClick={() => openDialog({ kind: "download", id })}
          >
            <RiDownloadLine data-icon="inline-start" />
            {t("downloads.title")}
          </Button>
          <Button variant="outline" onClick={() => attempt(() => containerAction(id, "restart"))}>
            <RiRestartLine data-icon="inline-start" />
            {t("containers.restart")}
          </Button>
          <Button
            variant="outline"
            onClick={() =>
              attempt(() =>
                showLog(t("containers.logsOf", { name: spec.name }), `containers/${id}/logs`)
              )
            }
          >
            <RiFileTextLine data-icon="inline-start" />
            {t("containers.logs")}
          </Button>
          <Button
            variant="outline"
            onClick={() =>
              attempt(() =>
                showLog(
                  t("containers.diagnosticsOf", { name: spec.name }),
                  `containers/${id}/diagnostics`
                )
              )
            }
          >
            <RiStethoscopeLine data-icon="inline-start" />
            {t("containers.diagnostics")}
          </Button>
          {running && (
            <Button variant="outline" onClick={() => attempt(() => containerAction(id, "stop"))}>
              <RiStopCircleLine data-icon="inline-start" />
              {t("containers.stop")}
            </Button>
          )}
          <Button variant="destructive" onClick={() => openDialog({ kind: "reset", id })}>
            <RiEraserLine data-icon="inline-start" />
            {t("reset.confirm")}
          </Button>
          <Button variant="destructive" onClick={() => openDialog({ kind: "delete", id })}>
            <RiDeleteBinLine data-icon="inline-start" />
            {t("common.delete")}
          </Button>
        </div>
        <SpecBadges spec={spec} />
        <LifecycleBadges item={item} />
        <GroupExposure item={item} />
        <Separator />
        <ErrorBoundary variant="section">
          <React.Suspense fallback={<Skeleton className="h-[140px] rounded-xl" />}>
            <ContainerUsage item={item} />
          </React.Suspense>
        </ErrorBoundary>
        <Separator />
        <ContainerActivity id={id} />
        <Separator />
        <dl className="grid grid-cols-2 gap-4">
          <Fact label={t("spec.resolution")}>{spec.resolution}</Fact>
          <Fact label={t("containers.sharedMemory")}>
            {t("spec.memory", { count: formatNumber(spec.shm_mb) })}
          </Fact>
          <Fact label={t("spec.internet")}>{yes(spec.internet)}</Fact>
          <Fact label={t("spec.flag.sudo")}>{yes(spec.sudo)}</Fact>
          <Fact label={t("table.created")}>{date(item.created_at)}</Fact>
          <Fact label={t("containers.dockerName")}>
            <code className="font-mono text-xs">{item.container}</code>
          </Fact>
          {(spec.groups?.length ?? 0) > 0 && (
            <>
              <Fact label={t("containers.reachedAs")}>
                <code className="font-mono text-xs">{formatList(groupNames(spec))}</code>
              </Fact>
              <Fact label={t("containers.groupMembers")}>
                {formatList(groupPeers(item, containers).map((peer) => peer.spec.name)) ||
                  t("containers.groupAlone")}
              </Fact>
            </>
          )}
          <div className="col-span-2">
            <Fact label={t("containers.image")}>
              <code className="font-mono text-xs">{spec.image}</code>
            </Fact>
          </div>
        </dl>
        <Separator />
        <div className="flex flex-col gap-2">
          <h3 className="text-xs text-muted-foreground">{t("nav.volumes")}</h3>
          {spec.mounts.length || setup?.mounts?.length ? (
            <ul className="flex flex-col gap-1">
              {spec.mounts.map((mount) => (
                <li key={mount.target} className="flex flex-wrap items-center gap-x-1 text-xs">
                  <code className="font-mono">{mount.volume}</code>
                  <span className="text-muted-foreground">
                    {rich(mount.readonly ? "volumes.mountedAtReadOnly" : "volumes.mountedAt", {
                      target: <code className="font-mono text-foreground">{mount.target}</code>,
                    })}
                  </span>
                </li>
              ))}
              {/* Chosen for the compose's volume parameters; the settings do not change them. */}
              {setup?.mounts?.map((mount) => (
                <li key={mount.target} className="flex flex-wrap items-center gap-x-1 text-xs">
                  <code className="font-mono">{mount.volume}</code>
                  <span className="text-muted-foreground">
                    {rich("volumes.mountedByCompose", {
                      target: <code className="font-mono text-foreground">{mount.target}</code>,
                    })}
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="text-xs text-muted-foreground">{t("containers.noMounts")}</p>
          )}
          <p className="text-xs text-muted-foreground">{t("spec.mountsHint")}</p>
        </div>
        {setup && setupStatus && (
          <>
            <Separator />
            <div className="flex flex-col gap-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <h3 className="text-xs text-muted-foreground">{t("compose.setup")}</h3>
                <StatusBadge
                  state={setupStatus}
                  tone={setupTone[setupStatus]}
                  label={setupLabel({ status: setupStatus })}
                />
              </div>
              {/* A compose of several steps shows each step, where it stands and its values. */}
              {setupGroups(setup).map(({ step, values, secrets }) => (
                <div key={step?.id ?? ""} className="flex flex-col gap-3">
                  {step && (
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <h4 className="text-sm font-medium">{step.name}</h4>
                      <StatusBadge
                        state={step.status}
                        tone={setupTone[step.status]}
                        label={
                          step.status === "pending"
                            ? t("compose.stepWaiting")
                            : setupLabel({ status: step.status })
                        }
                      />
                    </div>
                  )}
                  {values.length + secrets.length > 0 && (
                    <dl className="grid grid-cols-2 gap-4">
                      {values.map(([name, value]) => (
                        <Fact key={name} label={name}>
                          {/* Several choices arrive one per line. */}
                          <span className="whitespace-pre-wrap">
                            {value || (
                              <span className="text-muted-foreground">{t("compose.empty")}</span>
                            )}
                          </span>
                        </Fact>
                      ))}
                      {secrets.map((name) => (
                        <Fact key={name} label={name}>
                          <span className="text-muted-foreground">{t("compose.secret")}</span>
                        </Fact>
                      ))}
                    </dl>
                  )}
                </div>
              ))}
              {running && setupStatus === "failed" && (
                <Button
                  variant="outline"
                  className="self-start"
                  onClick={() => attempt(() => retrySetup(id))}
                >
                  <RiResetLeftLine data-icon="inline-start" />
                  {t("job.retry")}
                </Button>
              )}
            </div>
          </>
        )}
      </div>
      <DrawerFooter>
        <PrimaryAction item={item} />
        <DrawerClose asChild>
          <Button variant="outline">{t("common.done")}</Button>
        </DrawerClose>
      </DrawerFooter>
    </SideDrawer>
  )
}
