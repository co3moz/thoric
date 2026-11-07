import * as React from "react"
import { RiAddLine, RiSearchLine } from "@remixicon/react"

import { ViewTabs } from "@/components/data-table"
import { ConceptMedia } from "@/components/concept"
import { ContainerCard, PendingContainerCard } from "@/components/container-card"
import { ScaleSetSection } from "@/components/scale-set-section"
import { DockerAlert, ListFailed, listFailed, PageBody } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { openDialog } from "@/lib/dialogs"
import { needsAttention } from "@/lib/format"
import { t } from "@/lib/i18n"
import { pendingContainers } from "@/lib/pending"
import { useContainers, useImages, useJobs, useScaleSets } from "@/lib/queries"
import { bySet } from "@/lib/scale"
import type { ContainerItem } from "@/lib/types"

const views = {
  all: () => true,
  running: (item: ContainerItem) => item.state === "running",
  stopped: (item: ContainerItem) => item.state !== "running",
  attention: needsAttention,
} satisfies Record<string, (item: ContainerItem) => boolean>
type View = keyof typeof views

/** Every container as a card with its live preview, and those still being created first. */
export function ContainersPage() {
  const containers = useContainers()
  const images = useImages().data ?? []
  const jobs = useJobs().data ?? []
  const sets = useScaleSets().data ?? []
  const [view, setView] = React.useState<View>("all")
  const [search, setSearch] = React.useState("")
  const pending = pendingContainers(jobs, containers.data ?? [])
  const all = (containers.data ?? []).filter(
    (item) => !pending.some((entry) => entry.id === item.id)
  )
  const query = search.trim().toLowerCase()
  const matches = (name: string) => !query || name.toLowerCase().includes(query)
  const shown = all
    .filter(views[view])
    .filter((item) => matches(item.spec.name))
    .sort((a, b) => a.spec.name.localeCompare(b.spec.name))
  // A container being created is neither running nor stopped yet.
  const making = view === "all" ? pending.filter((entry) => matches(entry.name)) : []
  // The instances of a scale set are shown together under it, the other containers after them.
  const { groups, alone } = bySet(shown, sets)
  const count = (key: View) => all.filter(views[key]).length

  return (
    <PageBody>
      <DockerAlert />
      {containers.isPending ? (
        <div className="grid grid-cols-1 gap-4 px-4 lg:px-6 @xl/main:grid-cols-2 @5xl/main:grid-cols-3">
          {[0, 1, 2].map((index) => (
            <Card key={index} className="gap-4 p-3">
              <Skeleton className="aspect-video w-full" />
              <Skeleton className="h-5 w-1/2" />
              <Skeleton className="h-4 w-3/4" />
            </Card>
          ))}
        </div>
      ) : listFailed(containers) ? (
        <ListFailed query={containers} />
      ) : all.length === 0 && pending.length === 0 ? (
        <Empty className="mx-4 w-auto border lg:mx-6">
          <EmptyHeader>
            <ConceptMedia concept="container" />
            <EmptyTitle>{t("containers.none")}</EmptyTitle>
            <EmptyDescription>{t("containers.noneHint")}</EmptyDescription>
          </EmptyHeader>
          <EmptyContent>
            <Button onClick={() => openDialog({ kind: "create" })}>
              <RiAddLine data-icon="inline-start" />
              {t("containers.new")}
            </Button>
          </EmptyContent>
        </Empty>
      ) : (
        <Tabs value={view} onValueChange={(value) => setView(value as View)} className="gap-4">
          <div className="flex flex-wrap items-center justify-between gap-2 px-4 lg:px-6">
            <ViewTabs
              value={view}
              onChange={(value) => setView(value as View)}
              views={[
                { value: "all", label: t("common.all") },
                { value: "running", label: t("state.running"), count: count("running") },
                { value: "stopped", label: t("state.exited"), count: count("stopped") },
                {
                  value: "attention",
                  label: t("containers.view.attention"),
                  count: count("attention"),
                },
              ]}
            />
            <div className="relative">
              <RiSearchLine className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                type="search"
                aria-label={t("containers.filter")}
                placeholder={t("containers.filterPlaceholder")}
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                className="h-8 w-48 pl-8 sm:w-56"
              />
            </div>
          </div>
          <TabsContent value={view}>
            {shown.length || making.length ? (
              <div className="flex flex-col gap-6">
                {groups.map(({ set, instances }) => (
                  <ScaleSetSection key={set.name} set={set} instances={instances} images={images} />
                ))}
                {(alone.length > 0 || making.length > 0) && (
                  <div className="grid grid-cols-1 gap-4 px-4 lg:px-6 @xl/main:grid-cols-2 @5xl/main:grid-cols-3 @[100rem]/main:grid-cols-4">
                    {making.map((entry) => (
                      <PendingContainerCard key={entry.job.id} pending={entry} />
                    ))}
                    {alone.map((item) => (
                      <ContainerCard key={item.id} item={item} images={images} />
                    ))}
                  </div>
                )}
              </div>
            ) : (
              <p className="px-4 py-12 text-center text-sm text-muted-foreground lg:px-6">
                {t("containers.noneInView")}
              </p>
            )}
          </TabsContent>
        </Tabs>
      )}
    </PageBody>
  )
}
