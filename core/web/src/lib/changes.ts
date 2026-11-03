// A container's change list (`GET /api/containers/{id}/changes`): what changed in its own files
// since its Docker container was created, as the Files page's Changes view shows it.
import type { VolumeEntry } from "./volume-files"

export type ChangeKind = "added" | "changed" | "deleted"
export const CHANGE_KINDS: ChangeKind[] = ["added", "changed", "deleted"]

export type ChangeEntry = {
  path: string
  kind: ChangeKind
  /** The noise rule it matches, given only when the answer lists everything. */
  noise?: string
  type?: VolumeEntry["type"]
  size?: number
  modified_at?: number
}

export type ChangeCounts = { total: number } & Record<ChangeKind, number>

export type NoiseRule = { id: string; description: string; count: number; paths?: string[] }

export type ChangeMount = { path: string; type: string; volume?: string; read_only: boolean }

export type ChangeListing = {
  container_id: string
  running: boolean
  base: { image: string; image_id: string; commit: boolean; created_at: number | null }
  mounts: ChangeMount[]
  path: string
  summary: ChangeCounts & {
    folders: (ChangeCounts & { path: string })[]
    folders_truncated: boolean
  }
  noise: { left_out: boolean; count: number; rules: NoiseRule[] }
  total: number
  returned: number
  truncated: boolean
  details: "read" | "partial" | "failed" | "stopped"
  changes: ChangeEntry[]
}

/** What the Changes view asks for. An empty `path` is the whole container. */
export type ChangeFilter = { path: string; kinds: ChangeKind[]; all: boolean }

/** The most entries one answer lists, which "Show more" stops at. */
export const CHANGE_LIMIT = 5000

/** The route's path for `filter`, leaving out what the manager takes as its default. */
export function changesPath(id: string, filter: ChangeFilter, limit: number) {
  const query = new URLSearchParams()
  if (filter.path && filter.path !== "/") query.set("path", filter.path)
  if (filter.kinds.length && filter.kinds.length < CHANGE_KINDS.length)
    query.set("kinds", CHANGE_KINDS.filter((kind) => filter.kinds.includes(kind)).join(","))
  if (filter.all) query.set("all", "true")
  query.set("limit", String(limit))
  return `containers/${encodeURIComponent(id)}/changes?${query}`
}

/** A path's folder and its last name, which the list shows apart. */
export function splitChangePath(path: string) {
  const cut = path.lastIndexOf("/")
  return { folder: cut <= 0 ? "/" : path.slice(0, cut), name: path.slice(cut + 1) }
}

/**
 * The listed paths that are folders: what the desktop said is one, and each path the list holds an
 * entry below, which the manager puts right after it.
 */
export function folderPaths(entries: ChangeEntry[]) {
  const folders = new Set<string>()
  entries.forEach((entry, index) => {
    const next = entries[index + 1]
    if (entry.type === "directory" || next?.path.startsWith(`${entry.path}/`))
      folders.add(entry.path)
  })
  return folders
}

/**
 * Where the file explorer opens an entry: a folder as itself, anything else in its folder with
 * its name marked. A deleted entry is nowhere.
 */
export function explorerTarget(
  entry: ChangeEntry,
  folder: boolean
): { folder: string; name?: string } | null {
  if (entry.kind === "deleted") return null
  if (folder) return { folder: entry.path }
  const { folder: parent, name } = splitChangePath(entry.path)
  return { folder: parent, name }
}
