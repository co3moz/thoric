import * as React from "react"

import {
  columnHelper,
  DataTablePagination,
  DataTableView,
  SortHeader,
  useDataTable,
  ViewTabs,
  type DataColumn,
} from "@/components/data-table"
import { ListFailed, listFailed, PageBody } from "@/components/page"
import { ScriptsSwitch } from "@/components/scripts-switch"
import { StatusBadge } from "@/components/status-badge"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent } from "@/components/ui/tabs"
import { openDialog } from "@/lib/dialogs"
import {
  currentStage,
  duration,
  jobActive,
  jobName,
  jobProgress,
  jobTarget,
  jobTone,
  shortDate,
} from "@/lib/format"
import { t } from "@/lib/i18n"
import { useShowScripts } from "@/lib/job-history"
import { useContainers, useImages, useJobList } from "@/lib/queries"
import type { Job } from "@/lib/types"

type Row = Job & { target: string }
const column = columnHelper<Row>()

const columns: DataColumn<Row>[] = [
  column.accessor((job) => jobName(job.kind), {
    id: "action",
    header: () => t("jobs.column.action"),
    cell: ({ row, getValue }) => (
      <Button
        variant="link"
        className="h-auto p-0 text-foreground"
        onClick={() => openDialog({ kind: "job", id: row.original.id })}
      >
        {getValue()}
      </Button>
    ),
  }),
  column.accessor("target", {
    header: () => t("jobs.column.target"),
    cell: ({ getValue }) => (
      <span className="block max-w-56 truncate text-xs text-muted-foreground">
        {getValue() || t("common.none")}
      </span>
    ),
  }),
  column.accessor("status", {
    header: () => t("table.status"),
    cell: ({ row }) => {
      const job = row.original
      const percent = job.status === "running" ? jobProgress(job.progress)?.percent : undefined
      const stage = currentStage(job)
      return (
        <div className="flex items-center gap-2">
          <StatusBadge state={job.status} tone={jobTone(job.status)} />
          {stage && (
            <span className="max-w-48 truncate text-xs text-muted-foreground">{stage}</span>
          )}
          {percent !== undefined && (
            <span className="text-xs text-muted-foreground">{percent}%</span>
          )}
        </div>
      )
    },
  }),
  column.accessor("created_at", {
    header: ({ column }) => <SortHeader column={column}>{t("jobs.column.started")}</SortHeader>,
    cell: ({ row }) => (
      <span className="text-xs whitespace-nowrap text-muted-foreground">
        {shortDate(row.original.created_at)}
      </span>
    ),
  }),
  column.display({
    id: "duration",
    header: () => t("jobs.column.duration"),
    cell: ({ row }) => {
      const { created_at, finished_at } = row.original
      return (
        <span className="text-xs text-muted-foreground tabular-nums">
          {finished_at ? duration(finished_at - created_at) : t("jobs.view.active")}
        </span>
      )
    },
  }),
  column.display({
    id: "actions",
    cell: ({ row }) => (
      <div className="flex justify-end">
        <Button
          variant="outline"
          size="sm"
          onClick={() => openDialog({ kind: "job", id: row.original.id })}
        >
          {t("jobs.viewLog")}
        </Button>
      </div>
    ),
  }),
]

const views = {
  all: () => true,
  active: jobActive,
  failed: (job: Job) => job.status === "failed" || job.status === "interrupted",
} satisfies Record<string, (job: Job) => boolean>
type View = keyof typeof views

/** The latest 100 manager jobs, each with its log, with or without the scripts among them. */
export function JobsPage() {
  const jobs = useJobList(useShowScripts())
  const containers = useContainers().data
  const images = useImages().data
  const [view, setView] = React.useState<View>("all")
  const all = React.useMemo<Row[]>(() => {
    const names = new Map(containers?.map((container) => [container.id, container.spec.name]))
    const imageNames = new Map(images?.map((image) => [image.id, image.name]))
    return (jobs.data ?? []).map((job) => ({
      ...job,
      target: jobTarget(job, names, imageNames),
    }))
  }, [jobs.data, containers, images])
  const data = React.useMemo(() => all.filter(views[view]), [all, view])
  const table = useDataTable({
    data,
    columns,
    getRowId: (job) => job.id,
    sorting: [{ id: "created_at", desc: true }],
  })
  const change = (value: string) => {
    setView(value as View)
    table.setPageIndex(0)
  }

  return (
    <PageBody>
      <Tabs value={view} onValueChange={change} className="w-full flex-col justify-start gap-6">
        <div className="flex items-center justify-between gap-4 px-4 lg:px-6">
          <ViewTabs
            value={view}
            onChange={change}
            views={[
              { value: "all", label: t("jobs.view.all") },
              {
                value: "active",
                label: t("jobs.view.active"),
                count: all.filter(views.active).length,
              },
              {
                value: "failed",
                label: t("state.failed"),
                count: all.filter(views.failed).length,
              },
            ]}
          />
          <ScriptsSwitch />
        </div>
        <TabsContent
          value={view}
          className="relative flex flex-col gap-4 overflow-auto px-4 lg:px-6"
        >
          <DataTableView
            table={table}
            empty={
              jobs.isPending ? (
                t("jobs.loading")
              ) : listFailed(jobs) ? (
                <ListFailed query={jobs} row />
              ) : (
                t("jobs.noneInView")
              )
            }
          />
          <DataTablePagination table={table} />
        </TabsContent>
      </Tabs>
    </PageBody>
  )
}
