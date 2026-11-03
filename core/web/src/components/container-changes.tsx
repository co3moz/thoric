import * as React from "react"
import {
  RiAddCircleLine,
  RiCloseLine,
  RiDeleteBinLine,
  RiFolderOpenLine,
  RiPencilLine,
  RiRefreshLine,
} from "@remixicon/react"
import { keepPreviousData, useQuery } from "@tanstack/react-query"
import { cn } from "cn"

import { ConceptIcon } from "@/components/concept"
import { ListFailed } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { FieldError } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { api } from "@/lib/api"
import {
  CHANGE_KINDS,
  CHANGE_LIMIT,
  changesPath,
  explorerTarget,
  folderPaths,
  splitChangePath,
  type ChangeEntry,
  type ChangeKind,
  type ChangeListing,
  type ChangeMount,
} from "@/lib/changes"
import { canonicalFolder } from "@/lib/desktop-paths"
import { date, shortDate } from "@/lib/format"
import { formatNumber, has, plural, t } from "@/lib/i18n"
import type { ContainerItem } from "@/lib/types"
import { fileSize } from "@/lib/uploads"

const kindIcons: Record<ChangeKind, React.ReactNode> = {
  added: <RiAddCircleLine aria-hidden />,
  changed: <RiPencilLine aria-hidden />,
  deleted: <RiDeleteBinLine aria-hidden />,
}

/** The colors the activity log gives a new thing, a change and a deletion. */
const kindColors: Record<ChangeKind, string> = {
  added: "text-success",
  changed: "text-edit",
  deleted: "text-destructive",
}

/** What happened to a path, in its color beside its icon and its word. */
function KindBadge({ kind }: { kind: ChangeKind }) {
  return (
    <Badge variant="outline" className={cn("min-w-22 justify-start px-1.5", kindColors[kind])}>
      {kindIcons[kind]}
      {t(`changes.kind.${kind}`)}
    </Badge>
  )
}

/** A noise rule's name, or its id for a rule a newer manager added. */
const ruleLabel = (id: string) => {
  const key = `changes.rule.${id}`
  return has(key) ? t(key) : id
}

/** A mount the list does not cover, by its folder and what is mounted there. */
function MountBadge({ mount }: { mount: ChangeMount }) {
  return (
    <Badge variant="outline" className="max-w-full px-1.5 text-muted-foreground" title={mount.path}>
      {mount.type === "volume" ? <ConceptIcon concept="volume" /> : <RiFolderOpenLine />}
      <span className="truncate font-mono">{mount.path}</span>
      <span className="shrink-0">
        {mount.volume
          ? t("changes.mountVolume", { name: mount.volume })
          : t("changes.mountHostFolder")}
      </span>
    </Badge>
  )
}

/** Where the list starts, in words: when the Docker container was created, and from what. */
function baseText(listing: ChangeListing) {
  const { base } = listing
  if (base.commit)
    return base.created_at
      ? t("changes.baseCommit", { date: date(base.created_at) })
      : t("changes.baseCommitUndated")
  return base.created_at
    ? t("changes.base", { date: date(base.created_at), image: base.image })
    : t("changes.baseUndated", { image: base.image })
}

/** One entry; it renders again only when it changes, not as the folder field is typed in. */
const ChangeRow = React.memo(function ChangeRow({
  entry,
  isFolder,
  running,
  onOpen,
}: {
  entry: ChangeEntry
  isFolder: boolean
  running: boolean
  onOpen: (target: { folder: string; name?: string }) => void
}) {
  const { folder, name } = splitChangePath(entry.path)
  const target = explorerTarget(entry, isFolder)
  return (
    <li className={cn("flex items-center gap-2 px-3 py-2", entry.noise && "opacity-60")}>
      <KindBadge kind={entry.kind} />
      <span className="flex min-w-0 flex-1 flex-col" title={entry.path}>
        <span className="truncate font-mono text-sm">{name || entry.path}</span>
        <span className="truncate font-mono text-xs text-muted-foreground">{folder}</span>
      </span>
      {entry.noise && (
        <Badge variant="outline" className="hidden px-1.5 text-muted-foreground sm:inline-flex">
          {ruleLabel(entry.noise)}
        </Badge>
      )}
      <span className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground sm:block">
        {entry.size !== undefined ? fileSize(entry.size) : ""}
      </span>
      <span className="hidden w-32 shrink-0 text-right text-xs text-muted-foreground lg:block">
        {entry.modified_at !== undefined ? shortDate(entry.modified_at) : ""}
      </span>
      <Button
        variant="ghost"
        size="icon-sm"
        disabled={!target || !running}
        aria-label={t("changes.showInFiles", { name: name || entry.path })}
        title={
          !target
            ? t("changes.showDeleted")
            : running
              ? t("changes.showInFiles", { name: name || entry.path })
              : t("changes.showStopped")
        }
        onClick={() => target && onOpen(target)}
      >
        <RiFolderOpenLine />
      </Button>
    </li>
  )
})

/**
 * What changed in a container's own files since its Docker container was created, from Docker's
 * own list, without what the desktop changes on its own unless asked. A folder narrows it, the
 * kinds filter it, and a changed file opens in the file explorer while the container runs.
 */
export function ContainerChanges({
  container,
  active,
  onOpen,
}: {
  container: ContainerItem
  /** Whether the view shows; Docker walks the container's files for every list it reads. */
  active: boolean
  onOpen: (target: { folder: string; name?: string }) => void
}) {
  const [path, setPath] = React.useState("")
  const [address, setAddress] = React.useState("")
  const [kinds, setKinds] = React.useState<ChangeKind[]>(CHANGE_KINDS)
  const [all, setAll] = React.useState(false)
  const [limit, setLimit] = React.useState(500)
  const [error, setError] = React.useState("")
  const filter = { path, kinds, all }
  const listing = useQuery({
    queryKey: ["containers", container.id, "changes", filter, limit],
    queryFn: () => api<ChangeListing>(changesPath(container.id, filter, limit)),
    enabled: active,
    retry: false,
    // Read again on Refresh, or when the view opens after half a minute, never on window focus.
    staleTime: 30_000,
    refetchOnWindowFocus: false,
    // A new filter keeps the list it replaces in view until its own arrives.
    placeholderData: keepPreviousData,
    meta: { quiet: true },
  })

  const narrow = (folder: string) => {
    if (folder && !canonicalFolder(folder)) {
      setError(t("folders.notCanonical"))
      return
    }
    setError("")
    setPath(folder === "/" ? "" : folder)
    setAddress(folder === "/" ? "" : folder)
    setLimit(500)
  }

  const data = listing.isError ? undefined : listing.data
  const entries = React.useMemo(() => data?.changes ?? [], [data])
  const folders = React.useMemo(() => folderPaths(entries), [entries])
  const mounts = data?.mounts.filter((mount) => mount.type !== "tmpfs") ?? []
  const filtered = !!path || kinds.length < CHANGE_KINDS.length

  return (
    <div className="flex min-w-0 flex-col gap-3">
      {data && (
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div className="flex min-w-0 flex-col gap-1">
            <p className="text-sm break-words">{baseText(data)}</p>
            <p className="text-xs text-muted-foreground">{t("changes.scope")}</p>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={listing.isFetching}
            onClick={() => void listing.refetch()}
          >
            {listing.isFetching ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <RiRefreshLine data-icon="inline-start" />
            )}
            {t("changes.refresh")}
          </Button>
        </div>
      )}
      {mounts.length > 0 && (
        <div className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <span>{t("changes.notCovered")}</span>
          {mounts.map((mount) => (
            <MountBadge key={mount.path} mount={mount} />
          ))}
        </div>
      )}
      <form
        className="flex min-w-0 items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault()
          narrow(address.trim())
        }}
      >
        <Input
          aria-label={t("changes.folder")}
          placeholder={t("changes.everyFolder")}
          value={address}
          spellCheck={false}
          autoComplete="off"
          onChange={(event) => setAddress(event.target.value)}
          className="min-w-0 flex-1 font-mono"
        />
        {path && (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={t("changes.clearFolder")}
            title={t("changes.clearFolder")}
            onClick={() => narrow("")}
          >
            <RiCloseLine />
          </Button>
        )}
        <Button type="submit" variant="outline" size="sm">
          {t("changes.narrow")}
        </Button>
      </form>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <ToggleGroup
          type="multiple"
          variant="outline"
          size="sm"
          spacing={1}
          value={kinds}
          // One kind stays chosen; none would list nothing.
          onValueChange={(value: string[]) => {
            if (value.length) {
              setKinds(CHANGE_KINDS.filter((kind) => value.includes(kind)))
              setLimit(500)
            }
          }}
          aria-label={t("changes.kinds")}
        >
          {CHANGE_KINDS.map((kind) => (
            <ToggleGroupItem
              key={kind}
              value={kind}
              className="text-muted-foreground data-[state=on]:bg-muted data-[state=on]:text-foreground"
            >
              <span className={cn("contents", kinds.includes(kind) && kindColors[kind])}>
                {kindIcons[kind]}
              </span>
              {t(`changes.kind.${kind}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <label className="flex items-center gap-2 text-sm">
          <Switch
            size="sm"
            checked={all}
            onCheckedChange={(checked) => {
              setAll(checked)
              setLimit(500)
            }}
          />
          {t("changes.all")}
        </label>
      </div>
      <FieldError>{error}</FieldError>
      {data && data.summary.folders.length > 0 && (
        // One row that scrolls on a phone, as the explorer's places do, so the list stays in view.
        <nav
          aria-label={t("changes.folders")}
          className="-mx-1 flex min-w-0 gap-1.5 overflow-x-auto px-1 pb-1 sm:mx-0 sm:flex-wrap sm:overflow-visible sm:p-0"
        >
          {data.summary.folders.slice(0, 8).map((folder) => (
            <Button
              key={folder.path}
              variant="outline"
              size="sm"
              className="max-w-64 shrink-0 sm:max-w-full"
              title={folder.path}
              onClick={() => narrow(folder.path)}
            >
              <span className="truncate font-mono text-xs">{folder.path}</span>
              <span className="shrink-0 text-xs text-muted-foreground">
                {formatNumber(folder.total)}
              </span>
            </Button>
          ))}
        </nav>
      )}
      <div className="h-[max(16rem,calc(100svh-30rem))] overflow-y-auto rounded-lg border">
        {listing.isPending ? (
          <div className="flex flex-col gap-2 p-3">
            {[0, 1, 2].map((index) => (
              <Skeleton key={index} className="h-8" />
            ))}
          </div>
        ) : listing.isError ? (
          // Also a new filter's failure, while the list before it would still be in view.
          <ListFailed query={listing} className="h-full border-0" />
        ) : entries.length ? (
          <ul className="divide-y">
            {entries.map((entry) => (
              <ChangeRow
                key={entry.path}
                entry={entry}
                isFolder={folders.has(entry.path)}
                running={container.state === "running"}
                onOpen={onOpen}
              />
            ))}
          </ul>
        ) : (
          <p className="px-3 py-8 text-center text-sm text-muted-foreground">
            {filtered ? t("changes.noneMatching") : t("changes.none")}
          </p>
        )}
      </div>
      {data && (
        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
          <div className="flex min-w-0 flex-col gap-0.5">
            <span>
              {t("changes.counts", {
                total: formatNumber(data.summary.total),
                added: formatNumber(data.summary.added),
                changed: formatNumber(data.summary.changed),
                deleted: formatNumber(data.summary.deleted),
              })}
            </span>
            {data.noise.left_out && data.noise.count > 0 && (
              <span>
                {plural(data.noise.count, {
                  one: "changes.noiseLeftOut.one",
                  other: "changes.noiseLeftOut.other",
                })}
              </span>
            )}
            {data.details === "partial" && <span>{t("changes.detailsPartial")}</span>}
            {data.details === "failed" && <span>{t("changes.detailsFailed")}</span>}
          </div>
          {data.truncated && (
            <div className="flex items-center gap-2">
              <span>
                {t("changes.truncated", {
                  shown: formatNumber(data.returned),
                  total: formatNumber(data.total),
                })}
              </span>
              {limit < CHANGE_LIMIT && (
                <Button
                  variant="outline"
                  size="sm"
                  disabled={listing.isFetching}
                  onClick={() => setLimit(Math.min(limit * 2, CHANGE_LIMIT))}
                >
                  {t("changes.showMore")}
                </Button>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
