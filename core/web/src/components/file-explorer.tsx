import * as React from "react"
import {
  RiArrowLeftLine,
  RiArrowUpLine,
  RiBookmarkLine,
  RiComputerLine,
  RiDeleteBinLine,
  RiDownloadLine,
  RiFileLine,
  RiFileTextLine,
  RiFolderAddLine,
  RiFolderLine,
  RiHistoryLine,
  RiHome4Line,
  RiLinkM,
  RiMoreLine,
  RiPencilLine,
  RiUploadLine,
} from "@remixicon/react"
import { useQuery } from "@tanstack/react-query"
import { cn } from "cn"
import { toast } from "sonner"

import { CodeEditor } from "@/components/code-editor"
import { ConceptIcon } from "@/components/concept"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { FieldError } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Progress } from "@/components/ui/progress"
import { Skeleton } from "@/components/ui/skeleton"
import { useFileDrop } from "@/hooks/use-file-drop"
import { downloadFile, downloadFolder } from "@/lib/actions"
import { api, sessionExpired } from "@/lib/api"
import { useUnsaved } from "@/lib/dialogs"
import { canonicalFolder, HOME, placeGroups, type QuickFolder } from "@/lib/desktop-paths"
import { shortDate } from "@/lib/format"
import { codeLanguage, type CodeLanguage } from "@/lib/highlight"
import { formatNumber, rich, t } from "@/lib/i18n"
import { queryClient, useSettings } from "@/lib/queries"
import { FileUploads, fileSize, uploadStatus } from "@/lib/uploads"
import {
  EDIT_LIMIT,
  joinPath,
  parentPath,
  pathCrumbs,
  sortEntries,
  type VolumeEntry,
  type VolumeListing,
} from "@/lib/volume-files"
import { NameForm } from "./dialogs/name-form"

type Editing = { path: string; text: string; saved: string; language: CodeLanguage | null }
type Pending =
  | { kind: "folder" }
  | { kind: "rename"; entry: VolumeEntry }
  | { kind: "delete"; entry: VolumeEntry }

const icons: Record<VolumeEntry["type"], React.ReactNode> = {
  directory: <RiFolderLine className="size-4 shrink-0 text-primary" aria-hidden />,
  file: <RiFileLine className="size-4 shrink-0 text-muted-foreground" aria-hidden />,
  symlink: <RiLinkM className="size-4 shrink-0 text-muted-foreground" aria-hidden />,
  special: <RiFileLine className="size-4 shrink-0 text-muted-foreground" aria-hidden />,
}

const homeIcons: Record<string, React.ReactNode> = {
  [HOME]: <RiHome4Line aria-hidden />,
  [`${HOME}/Desktop`]: <RiComputerLine aria-hidden />,
  [`${HOME}/Documents`]: <RiFileTextLine aria-hidden />,
  [`${HOME}/Downloads`]: <RiDownloadLine aria-hidden />,
}

const kindIcons: Record<Exclude<QuickFolder["kind"], "home">, React.ReactNode> = {
  compose: <RiBookmarkLine aria-hidden />,
  volume: <ConceptIcon concept="volume" />,
  recent: <RiHistoryLine aria-hidden />,
}

/** The upload queue of the open folder, rendered again whenever it reports progress. */
function useUploads(onDone: () => void) {
  const [, render] = React.useReducer((count: number) => count + 1, 0)
  const [uploads] = React.useState(
    () =>
      new FileUploads({
        createRequest: () => new XMLHttpRequest(),
        onChange: render,
        onUnauthorized: sessionExpired,
      })
  )
  React.useEffect(() => () => uploads.cancel(), [uploads])
  // Leaving the page, or closing the dialog the explorer is in, asks first while files upload.
  useUnsaved(uploads.running && "upload")
  const running = React.useRef(false)
  React.useEffect(() => {
    if (running.current && !uploads.running) onDone()
    running.current = uploads.running
  })
  return uploads
}

/** A folder the explorer is asked to open, and the entry of it to mark. */
export type Reveal = { folder: string; name?: string; at: number }

export type ExplorerProps = {
  /** The API path whose `files/*` routes hold the files: `containers/{id}` or `volumes/{name}`. */
  base: string
  /** The query key the listings are cached under; a folder's path is appended. */
  queryKey: readonly unknown[]
  /** How the breadcrumb names `/`. */
  rootName: string
  /** The folder shown first. */
  start?: string
  /**
   * A folder to open with an entry of it marked, such as one the change list shows; a new `at`
   * asks again. Open edits stay, with the question whether to discard them.
   */
  reveal?: Reveal
  /** Folders listed beside the files, as the upload dialog lists them for a container. */
  places?: QuickFolder[]
  /** Why files cannot be uploaded and folders not created in a folder, or an empty string. */
  writeProblem?: (path: string) => string
  /** A listing failure in words, from the manager's message. */
  describeFailure?: (message: string) => string
  /** What an empty `/` says. */
  emptyRoot?: string
  /** Classes for the list and the editor, which set their height. */
  paneClassName?: string
  /** Whether the explorer is shown; a closed dialog stops its uploads and its listing. */
  active?: boolean
}

/**
 * Browses the files of a container or a volume as the desktop user and changes them: folders
 * open in place, a file up to 512 KiB opens as text and saves back, larger ones download, and
 * files dropped on the list upload into the open folder. Every change goes through the same
 * manager routes the API documents, which make the decisions; this only says why first.
 */
export function FileExplorer({
  base,
  queryKey,
  rootName,
  start = "/",
  reveal,
  places,
  writeProblem = () => "",
  describeFailure = (message) => message,
  emptyRoot,
  paneClassName = "h-[min(26rem,50vh)]",
  active = true,
}: ExplorerProps) {
  const limit = useSettings().data?.upload_limit_bytes ?? 0
  const [path, setPath] = React.useState(reveal?.folder ?? start)
  const [address, setAddress] = React.useState(reveal?.folder ?? start)
  const [editing, setEditing] = React.useState<Editing | null>(null)
  const [pending, setPending] = React.useState<Pending | null>(null)
  const [discarding, setDiscarding] = React.useState(false)
  const [mark, setMark] = React.useState(reveal)
  // A new request opens its folder behind an open file, which closes unless it has edits, and
  // those it asks about: neither they nor the uploads are lost.
  if (reveal && reveal.at !== mark?.at) {
    setMark(reveal)
    setPath(reveal.folder)
    setAddress(reveal.folder)
    setPending(null)
    if (editing && editing.text !== editing.saved) setDiscarding(true)
    else setEditing(null)
  }
  const [busy, setBusy] = React.useState(false)
  const [error, setError] = React.useState("")
  const listing = useQuery({
    queryKey: [...queryKey, path],
    queryFn: () => api<VolumeListing>(`${base}/files/list`, { path, limit: 2000 }),
    enabled: active,
    retry: false,
    // The list says why it is empty, so the failure needs no toast.
    meta: { quiet: true },
  })
  const marked = React.useRef<HTMLLIElement>(null)
  React.useEffect(() => {
    marked.current?.scrollIntoView({ block: "nearest" })
  }, [listing.data, mark])
  const refresh = () => queryClient.invalidateQueries({ queryKey })
  const uploads = useUploads(() => void refresh())
  const input = React.useRef<HTMLInputElement>(null)
  React.useEffect(() => {
    if (!active) uploads.cancel()
  }, [active, uploads])

  const failure = listing.error ? describeFailure((listing.error as Error).message) : ""
  const problem = writeProblem(path)
  const writable = !problem && !failure && !listing.isPending

  const go = (target: string) => {
    if (!canonicalFolder(target)) {
      setError(t("folders.notCanonical"))
      return
    }
    setPath(target)
    setAddress(target)
    setPending(null)
    setError("")
  }

  const upload = (files: File[]) => {
    if (!writable) {
      setError(problem || failure)
      return
    }
    try {
      uploads.select(files, limit)
      setError("")
      void uploads.start({ target: base, destination: path, replace: false })
    } catch (refusal) {
      setError((refusal as Error).message)
    }
  }
  const drop = useFileDrop(upload)

  const run = async (action: () => Promise<unknown>, success?: string) => {
    setBusy(true)
    setError("")
    try {
      await action()
      setPending(null)
      if (success) toast.success(success)
      await refresh()
    } catch (refusal) {
      setError((refusal as Error).message)
    } finally {
      setBusy(false)
    }
  }

  const openEntry = async (entry: VolumeEntry) => {
    const target = joinPath(path, entry.name)
    if (entry.type === "directory") return go(target)
    if (entry.type !== "file") return
    if (entry.size > EDIT_LIMIT) {
      toast.info(t("files.tooLarge", { name: entry.name, size: fileSize(EDIT_LIMIT) }))
      downloadFile(base, target, entry.size, limit)
      return
    }
    setBusy(true)
    setError("")
    try {
      const file = await api<{ data: string }>(`${base}/files/read`, {
        path: target,
        limit: EDIT_LIMIT,
      })
      setEditing({
        path: target,
        text: file.data,
        saved: file.data,
        language: codeLanguage(target, file.data),
      })
    } catch (refusal) {
      setError((refusal as Error).message)
    } finally {
      setBusy(false)
    }
  }

  const save = () => {
    if (!editing) return
    const { path: target, text } = editing
    return run(
      async () => {
        await api(`${base}/files/write`, { path: target, data: text, replace: true })
        // Typing goes on while the file saves: what was sent is saved, and the text stays as it is.
        setEditing((current) => (current?.path === target ? { ...current, saved: text } : current))
      },
      t("files.saved", { path: target })
    )
  }

  const entries = sortEntries(listing.data?.items ?? [])
  const changed = !!editing && editing.text !== editing.saved
  const queue = uploads.items

  if (editing) {
    return (
      <div className="flex min-w-0 flex-col gap-3">
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="icon-sm"
            aria-label={t("files.backToFolder")}
            title={t("files.backToFolder")}
            onClick={() => (changed ? setDiscarding(true) : setEditing(null))}
          >
            <RiArrowLeftLine />
          </Button>
          <code className="truncate font-mono text-sm">{editing.path}</code>
        </div>
        {discarding && (
          <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-destructive/40 p-3 text-sm">
            <span>{t("files.discardQuestion")}</span>
            <div className="flex gap-2">
              <Button
                variant="destructive"
                size="sm"
                onClick={() => {
                  setDiscarding(false)
                  setEditing(null)
                }}
              >
                {t("files.discard")}
              </Button>
              <Button variant="outline" size="sm" onClick={() => setDiscarding(false)}>
                {t("files.keepEditing")}
              </Button>
            </div>
          </div>
        )}
        <CodeEditor
          fill
          label={t("files.contentsOf", { path: editing.path })}
          language={editing.language ?? undefined}
          path={editing.path}
          value={editing.text}
          onChange={(text) => setEditing((current) => current && { ...current, text })}
          onSave={() => changed && !busy && void save()}
          className={paneClassName}
        />
        <FieldError>{error}</FieldError>
        <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
          <Button
            variant="outline"
            onClick={() => downloadFile(base, editing.path, 0, limit)}
            disabled={busy || changed}
          >
            <RiDownloadLine data-icon="inline-start" />
            {t("files.download")}
          </Button>
          <Button onClick={save} disabled={busy || !changed}>
            {busy ? t("files.saving") : t("common.save")}
          </Button>
        </div>
      </div>
    )
  }

  const crumbs = pathCrumbs(path)
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <form
          className="flex min-w-0 flex-1 basis-72 items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            go(address.trim())
          }}
        >
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label={t("folders.up")}
            title={t("folders.up")}
            disabled={path === "/"}
            onClick={() => go(parentPath(path))}
          >
            <RiArrowUpLine />
          </Button>
          <Input
            aria-label={t("folders.address")}
            value={address}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => setAddress(event.target.value)}
            className="min-w-0 flex-1 font-mono"
          />
          <Button type="submit" variant="outline" size="sm">
            {t("folders.open")}
          </Button>
        </form>
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={busy || uploads.running || !writable}
            title={problem || undefined}
            onClick={() => setPending({ kind: "folder" })}
          >
            <RiFolderAddLine data-icon="inline-start" />
            {t("files.newFolder")}
          </Button>
          <Button
            size="sm"
            disabled={uploads.running || !writable}
            title={problem || undefined}
            onClick={() => input.current?.click()}
          >
            <RiUploadLine data-icon="inline-start" />
            {t("files.upload")}
          </Button>
          <input
            ref={input}
            type="file"
            multiple
            hidden
            onChange={(event) => {
              upload([...(event.target.files ?? [])])
              event.target.value = ""
            }}
          />
        </div>
      </div>
      {pending?.kind === "folder" && (
        <NameForm
          label={t("files.folderName")}
          initial=""
          busy={busy}
          onCancel={() => setPending(null)}
          onSubmit={(folder) =>
            run(
              () => api(`${base}/files/mkdir`, { path: joinPath(path, folder) }),
              t("files.created", { name: folder })
            )
          }
        />
      )}
      {pending?.kind === "rename" && (
        <NameForm
          label={t("files.newName", { name: pending.entry.name })}
          initial={pending.entry.name}
          busy={busy}
          onCancel={() => setPending(null)}
          onSubmit={(renamed) =>
            run(
              () =>
                api(`${base}/files/move`, {
                  source: joinPath(path, pending.entry.name),
                  destination: joinPath(path, renamed),
                }),
              t("files.renamed", { name: pending.entry.name, renamed })
            )
          }
        />
      )}
      {pending?.kind === "delete" && (
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-destructive/40 p-3 text-sm">
          <span>
            {rich(
              pending.entry.type === "directory"
                ? "files.deleteFolderQuestion"
                : "files.deleteQuestion",
              { name: <strong className="font-medium">{pending.entry.name}</strong> }
            )}
          </span>
          <div className="flex gap-2">
            <Button
              variant="destructive"
              size="sm"
              disabled={busy}
              onClick={() =>
                run(
                  () =>
                    api(`${base}/files/delete`, {
                      path: joinPath(path, pending.entry.name),
                      recursive: pending.entry.type === "directory",
                    }),
                  t("files.deleted", { name: pending.entry.name })
                )
              }
            >
              {t("common.delete")}
            </Button>
            <Button variant="outline" size="sm" disabled={busy} onClick={() => setPending(null)}>
              {t("common.cancel")}
            </Button>
          </div>
        </div>
      )}
      <div
        className={cn(
          "grid min-w-0 gap-3",
          places?.length && "md:grid-cols-[minmax(0,11rem)_minmax(0,1fr)]"
        )}
      >
        {!!places?.length && (
          <nav
            aria-label={t("folders.places")}
            className="-mx-1 flex min-w-0 gap-1 overflow-x-auto px-1 pb-1 md:mx-0 md:flex-col md:overflow-visible md:p-0"
          >
            {placeGroups(places).map((group) => (
              <div
                key={group.places[0].path}
                className="flex shrink-0 gap-1 md:flex-col md:gap-0.5 md:pt-3 md:first:pt-0"
              >
                <span
                  title={group.step}
                  className="hidden truncate px-2 pb-0.5 text-xs text-muted-foreground md:block"
                >
                  {group.step ?? t(`folders.group.${group.kind}`)}
                </span>
                {group.places.map((place) => (
                  <Button
                    key={place.path}
                    type="button"
                    variant={place.path === path ? "secondary" : "ghost"}
                    size="sm"
                    aria-current={place.path === path ? "location" : undefined}
                    title={place.path}
                    onClick={() => go(place.path)}
                    className="max-w-56 justify-start md:max-w-none"
                  >
                    {place.kind === "home" ? homeIcons[place.path] : kindIcons[place.kind]}
                    <span
                      className={cn("truncate", place.kind === "recent" && "font-mono text-xs")}
                    >
                      {place.label}
                    </span>
                  </Button>
                ))}
              </div>
            ))}
          </nav>
        )}
        <div
          {...drop.props}
          className={cn(
            "flex min-w-0 flex-col gap-2 rounded-lg",
            drop.over && writable && "ring-2 ring-primary ring-offset-4 ring-offset-background"
          )}
        >
          <nav
            aria-label={t("files.folder")}
            className="flex min-w-0 flex-wrap items-center text-sm"
          >
            {crumbs.map((crumb, index) => (
              <React.Fragment key={crumb.path}>
                {index > 1 && <span className="px-0.5 text-muted-foreground">/</span>}
                <button
                  type="button"
                  className={cn(
                    "rounded px-1 font-mono break-all hover:underline",
                    index === crumbs.length - 1 && "font-medium"
                  )}
                  aria-current={index === crumbs.length - 1 ? "location" : undefined}
                  onClick={() => go(crumb.path)}
                >
                  {index === 0 ? rootName : crumb.name}
                </button>
              </React.Fragment>
            ))}
          </nav>
          <div className={cn("overflow-y-auto rounded-lg border", paneClassName)}>
            {listing.isPending ? (
              <div className="flex flex-col gap-2 p-3">
                {[0, 1, 2].map((index) => (
                  <Skeleton key={index} className="h-6" />
                ))}
              </div>
            ) : failure ? (
              <p className="px-3 py-8 text-center text-sm text-muted-foreground">{failure}</p>
            ) : (
              <ul className="divide-y">
                {path !== "/" && (
                  <li>
                    <button
                      type="button"
                      className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-muted/50"
                      onClick={() => go(parentPath(path))}
                    >
                      <RiFolderLine className="size-4 text-muted-foreground" aria-hidden />
                      <span className="font-mono">..</span>
                    </button>
                  </li>
                )}
                {entries.map((entry) => (
                  <li
                    key={entry.name}
                    ref={path === mark?.folder && entry.name === mark.name ? marked : undefined}
                    className={cn(
                      "flex items-center gap-2 pr-2",
                      path === mark?.folder && entry.name === mark.name && "bg-muted"
                    )}
                  >
                    <button
                      type="button"
                      disabled={busy || entry.type === "symlink" || entry.type === "special"}
                      className="flex min-w-0 flex-1 items-center gap-2 px-3 py-2 text-left text-sm hover:bg-muted/50 disabled:cursor-default disabled:hover:bg-transparent"
                      onClick={() => void openEntry(entry)}
                      title={
                        entry.type === "file" && entry.size <= EDIT_LIMIT
                          ? t("files.openAsText")
                          : entry.type === "symlink"
                            ? t("folders.symlink")
                            : undefined
                      }
                    >
                      {icons[entry.type]}
                      <span className="truncate font-mono">{entry.name}</span>
                      {entry.type === "file" && entry.size <= EDIT_LIMIT && (
                        <RiFileTextLine
                          className="hidden size-3.5 shrink-0 text-muted-foreground sm:block"
                          aria-hidden
                        />
                      )}
                    </button>
                    <span className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground sm:block">
                      {entry.type === "file" ? fileSize(entry.size) : ""}
                    </span>
                    <span className="hidden w-32 shrink-0 text-right text-xs text-muted-foreground lg:block">
                      {shortDate(entry.modified_at)}
                    </span>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          disabled={busy}
                          aria-label={t("containers.actionsOf", { name: entry.name })}
                        >
                          <RiMoreLine />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-auto">
                        {entry.type === "file" && (
                          <DropdownMenuItem
                            tone="transfer"
                            onSelect={() =>
                              downloadFile(base, joinPath(path, entry.name), entry.size, limit)
                            }
                          >
                            <RiDownloadLine />
                            {t("files.download")}
                          </DropdownMenuItem>
                        )}
                        {entry.type === "directory" && (
                          <DropdownMenuItem
                            tone="transfer"
                            onSelect={() => downloadFolder(base, joinPath(path, entry.name))}
                          >
                            <RiDownloadLine />
                            {t("files.downloadArchive")}
                          </DropdownMenuItem>
                        )}
                        <DropdownMenuItem
                          tone="edit"
                          onSelect={() => setPending({ kind: "rename", entry })}
                        >
                          <RiPencilLine />
                          {t("files.rename")}
                        </DropdownMenuItem>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          variant="destructive"
                          onSelect={() => setPending({ kind: "delete", entry })}
                        >
                          <RiDeleteBinLine />
                          {t("common.delete")}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </li>
                ))}
                {!entries.length && (
                  <li className="px-3 py-8 text-center text-sm text-muted-foreground">
                    {path === "/" && emptyRoot ? emptyRoot : t("files.folderEmpty")}{" "}
                    {writable && t("files.dropToUpload")}
                  </li>
                )}
              </ul>
            )}
          </div>
        </div>
      </div>
      <FieldError>{error}</FieldError>
      {listing.data?.truncated && (
        <p className="text-xs text-muted-foreground">
          {t("files.truncated", {
            shown: formatNumber(listing.data.items.length),
            total: formatNumber(listing.data.total),
          })}
        </p>
      )}
      {problem && !failure && (
        <p role="status" aria-live="polite" className="text-xs text-muted-foreground">
          {problem}
        </p>
      )}
      {queue.length > 0 && (
        <ul className="flex max-h-40 flex-col gap-2 overflow-y-auto">
          {queue.map((entry, index) => (
            <li key={`${entry.name}-${index}`} className="flex flex-col gap-1">
              <div className="flex items-baseline justify-between gap-2 text-xs">
                <span className="truncate font-mono">{entry.name}</span>
                <span className="shrink-0 text-muted-foreground">
                  {entry.error || `${fileSize(entry.size)} · ${uploadStatus(entry.status)}`}
                </span>
              </div>
              <Progress
                value={(entry.loaded * 100) / Math.max(entry.size, 1)}
                aria-label={t("uploads.progressOf", { name: entry.name })}
              />
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
