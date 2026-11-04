import * as React from "react"
import { RiFileHistoryLine, RiFolderOpenLine, RiPlayLine } from "@remixicon/react"

import { ConceptIcon } from "@/components/concept"
import { ContainerChanges } from "@/components/container-changes"
import { FileExplorer, type Reveal } from "@/components/file-explorer"
import { DockerAlert, ListFailed, listFailed, PageBody, SectionHeading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { attempt, containerAction } from "@/lib/actions"
import {
  HOME,
  listingProblem,
  quickFolders,
  recentFolders,
  uploadProblem,
} from "@/lib/desktop-paths"
import { stateLabel } from "@/lib/format"
import { t } from "@/lib/i18n"
import { useContainers } from "@/lib/queries"
import { href, useRoute } from "@/lib/route"
import { DESKTOP_FILE } from "@/lib/script-completions"
import type { ContainerItem } from "@/lib/types"

const byName = (a: ContainerItem, b: ContainerItem) => a.spec.name.localeCompare(b.spec.name)

/** A running container's files as the desktop user sees them, with its usual folders beside. */
function ContainerFiles({ container, reveal }: { container: ContainerItem; reveal?: Reveal }) {
  const [recent] = React.useState(() => recentFolders(container.id))
  return (
    <FileExplorer
      base={`containers/${encodeURIComponent(container.id)}`}
      queryKey={["containers", container.id, "files"]}
      rootName="/"
      start={HOME}
      reveal={reveal}
      places={quickFolders(container, recent)}
      writeProblem={(path) => uploadProblem(container, path)}
      describeFailure={listingProblem}
      paneClassName="h-[max(16rem,calc(100svh-22rem))]"
      scripts={DESKTOP_FILE}
    />
  )
}

/** The chosen container's files, or why they cannot be browsed now. */
function FilesView({ chosen, reveal }: { chosen: ContainerItem; reveal?: Reveal }) {
  if (chosen.state === "running")
    return (
      <Card>
        <CardContent>
          <ContainerFiles key={chosen.id} container={chosen} reveal={reveal} />
        </CardContent>
      </Card>
    )
  return (
    <Empty className="border">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <RiFolderOpenLine />
        </EmptyMedia>
        <EmptyTitle>{t("filesPage.notRunning", { name: chosen.spec.name })}</EmptyTitle>
        <EmptyDescription>{t("filesPage.notRunningHint")}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button size="sm" onClick={() => attempt(() => containerAction(chosen.id, "start"))}>
          <RiPlayLine data-icon="inline-start" />
          {t("containers.start")}
        </Button>
      </EmptyContent>
    </Empty>
  )
}

/**
 * The Files page: pick a container, then browse, open and edit its files while it runs, or see
 * what changed in them, also while it is stopped. The chosen container and the view are part of
 * the address, so a link or a reload opens the same one.
 */
export function FilesPage() {
  const route = useRoute()
  const chosenId = route.page === "files" ? route.container : undefined
  const view = route.page === "files" && route.view === "changes" ? "changes" : "files"
  const containers = useContainers()
  const all = [...(containers.data ?? [])].sort(byName)
  const chosen = all.find((item) => item.id === chosenId)
  // What the change list asked the explorer to open, for the container it belongs to.
  const [reveal, setReveal] = React.useState<Reveal & { container: string }>()
  const at = (id: string, next: "files" | "changes") =>
    href({ page: "files", container: id, ...(next === "changes" ? { view: next } : {}) })

  // Without a choice in the address, show the first running container, or the first there is,
  // whose change list can be read while it is stopped.
  React.useEffect(() => {
    const first = all.find((item) => item.state === "running") ?? all[0]
    if (!chosenId && first) location.replace(at(first.id, view))
  })

  return (
    <PageBody>
      <DockerAlert />
      <SectionHeading title={t("filesPage.title")} description={t("filesPage.description")}>
        {all.length > 0 && (
          <Select
            value={chosen ? chosen.id : ""}
            onValueChange={(id) => location.assign(at(id, view))}
          >
            <SelectTrigger aria-label={t("filesPage.container")} className="w-64 max-w-full">
              <SelectValue placeholder={t("filesPage.choose")} />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {all.map((item) => (
                  <SelectItem key={item.id} value={item.id}>
                    <ConceptIcon concept="container" />
                    {item.spec.name}
                    {item.state !== "running" && (
                      <span className="text-muted-foreground">{stateLabel(item.state)}</span>
                    )}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        )}
      </SectionHeading>
      <div className="px-4 lg:px-6">
        {containers.isPending ? (
          <Skeleton className="h-96 rounded-2xl" />
        ) : listFailed(containers) ? (
          <ListFailed query={containers} className="border" />
        ) : chosen ? (
          <Tabs
            value={view}
            onValueChange={(next) => location.assign(at(chosen.id, next as "files" | "changes"))}
          >
            <TabsList>
              <TabsTrigger value="files">
                <RiFolderOpenLine />
                {t("filesPage.files")}
              </TabsTrigger>
              <TabsTrigger value="changes">
                <RiFileHistoryLine />
                {t("filesPage.changes")}
              </TabsTrigger>
            </TabsList>
            {/* Both stay mounted, so a folder open in one and a filter of the other survive. */}
            <TabsContent value="files" forceMount className="data-[state=inactive]:hidden">
              <FilesView
                chosen={chosen}
                reveal={reveal?.container === chosen.id ? reveal : undefined}
              />
            </TabsContent>
            <TabsContent value="changes" forceMount className="data-[state=inactive]:hidden">
              <Card>
                <CardContent>
                  <ContainerChanges
                    key={chosen.id}
                    container={chosen}
                    active={view === "changes"}
                    onOpen={(target) => {
                      setReveal({ container: chosen.id, ...target, at: Date.now() })
                      location.assign(at(chosen.id, "files"))
                    }}
                  />
                </CardContent>
              </Card>
            </TabsContent>
          </Tabs>
        ) : (
          <Empty className="border">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <RiFolderOpenLine />
              </EmptyMedia>
              <EmptyTitle>
                {chosenId ? t("filesPage.missing") : t("filesPage.noneRunning")}
              </EmptyTitle>
              <EmptyDescription>{t("filesPage.noneRunningHint")}</EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button size="sm" variant="outline" asChild>
                <a href={href({ page: "containers" })}>{t("nav.containers")}</a>
              </Button>
            </EmptyContent>
          </Empty>
        )}
      </div>
    </PageBody>
  )
}
