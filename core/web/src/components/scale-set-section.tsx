import { RiAddLine, RiDeleteBinLine, RiSettings3Line, RiStackLine } from "@remixicon/react"

import { ContainerCard } from "@/components/container-card"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { openDialog } from "@/lib/dialogs"
import { t } from "@/lib/i18n"
import { room, type ScaleSet } from "@/lib/scale"
import type { ContainerItem, ImageItem } from "@/lib/types"

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
  const full = room(set) === 0
  return (
    <section className="flex flex-col gap-3" aria-label={t("scale.setOf", { name: set.name })}>
      <div className="flex flex-wrap items-center justify-between gap-2 px-4 lg:px-6">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <RiStackLine className="size-4 shrink-0 text-muted-foreground" aria-hidden />
          <h2 className="text-sm font-medium">{set.name}</h2>
          <Badge variant="outline">
            {t("scale.count", { count: set.instances.length, max: set.max })}
          </Badge>
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
      <div className="grid grid-cols-1 gap-4 px-4 lg:px-6 @xl/main:grid-cols-2 @5xl/main:grid-cols-3 @[100rem]/main:grid-cols-4">
        {instances.map((item) => (
          <ContainerCard key={item.id} item={item} images={images} />
        ))}
      </div>
    </section>
  )
}
