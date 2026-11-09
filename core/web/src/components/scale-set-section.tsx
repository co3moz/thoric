import {
  RiAddLine,
  RiDeleteBinLine,
  RiPulseLine,
  RiSettings3Line,
  RiStackLine,
} from "@remixicon/react"

import { CallTime, useNow } from "@/components/activity-feed"
import { ContainerCard } from "@/components/container-card"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { openDialog } from "@/lib/dialogs"
import { plural, t } from "@/lib/i18n"
import { useScaleSetActivity } from "@/lib/queries"
import { navigate } from "@/lib/route"
import { activeInstances, room, type ScaleSet } from "@/lib/scale"
import type { ContainerItem, ImageItem } from "@/lib/types"

/**
 * What agents did in each instance of a set, from the activity log's kept rows: its calls, how
 * many of them failed and when the last was. An instance opens the Activity page for it. Nothing
 * shows while no instance has taken a call.
 */
function SetActivity({ name }: { name: string }) {
  const now = useNow()
  const instances = activeInstances(useScaleSetActivity(name).data)
  if (!instances.length) return null
  return (
    <ul
      className="flex flex-wrap items-center gap-x-1 gap-y-1 px-4 text-xs text-muted-foreground lg:px-6"
      aria-label={t("scale.activity", { name })}
    >
      <li className="flex items-center" aria-hidden>
        <RiPulseLine className="size-3.5" />
      </li>
      {instances.map((item) => (
        <li key={item.container_id}>
          <button
            type="button"
            title={t("scale.activity.open", { number: item.instance })}
            onClick={() => navigate({ page: "activity", container: item.container_id })}
            className="inline-flex items-center gap-1.5 rounded-md px-1.5 py-0.5 tabular-nums transition-colors outline-none hover:bg-muted hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
          >
            <span className="font-medium text-foreground">
              {t("scale.activity.instance", { number: item.instance })}
            </span>
            <span>
              {plural(item.calls, {
                one: "scale.activity.calls.one",
                other: "scale.activity.calls.other",
              })}
            </span>
            {item.errors > 0 && (
              <span className="text-destructive">
                {plural(item.errors, {
                  one: "scale.activity.errors.one",
                  other: "scale.activity.errors.other",
                })}
              </span>
            )}
            {item.last_at !== null && <CallTime at={item.last_at} now={now} />}
          </button>
        </li>
      ))}
    </ul>
  )
}

/**
 * A scale set on the containers page: its name, how many instances it has of how many, its MCP
 * endpoint and its actions, above the cards of its instances that the view shows.
 */
export function ScaleSetSection({
  set,
  instances,
  images,
}: {
  set: ScaleSet
  instances: ContainerItem[]
  images: ImageItem[]
}) {
  const full = room(set) === 0 || set.deleting === true
  return (
    <section className="flex flex-col gap-3" aria-label={t("scale.setOf", { name: set.name })}>
      <div className="flex flex-wrap items-center justify-between gap-2 px-4 lg:px-6">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <RiStackLine className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          <h2 className="text-sm font-medium">{set.name}</h2>
          <Badge variant="outline">
            {t("scale.count", { count: set.instances.length, max: set.max })}
          </Badge>
          {set.deleting && <Badge variant="secondary">{t("scale.deleting")}</Badge>}
          {set.endpoint_path && (
            <code className="font-mono text-xs text-muted-foreground">{set.endpoint_path}</code>
          )}
        </div>
        <div className="flex items-center gap-1">
          <Button
            size="sm"
            variant="outline"
            disabled={full}
            title={full ? t("scale.full", { max: set.max }) : undefined}
            onClick={() => openDialog({ kind: "scale-instance", name: set.name })}
          >
            <RiAddLine data-icon="inline-start" />
            {t("scale.addInstance")}
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("scale.settingsTitle", { name: set.name })}
            onClick={() => openDialog({ kind: "scale-set", name: set.name })}
          >
            <RiSettings3Line />
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            className="text-destructive"
            aria-label={t("scale.deleteTitle", { name: set.name })}
            onClick={() => openDialog({ kind: "scale-set-delete", name: set.name })}
          >
            <RiDeleteBinLine />
          </Button>
        </div>
      </div>
      <SetActivity name={set.name} />
      <div className="grid grid-cols-1 gap-4 px-4 lg:px-6 @xl/main:grid-cols-2 @5xl/main:grid-cols-3 @[100rem]/main:grid-cols-4">
        {instances.map((item) => (
          <ContainerCard key={item.id} item={item} images={images} />
        ))}
      </div>
    </section>
  )
}
