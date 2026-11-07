import { sessionExpired } from "./api"
import { ArchiveImport } from "./archives"
import type { UploadRequest } from "./uploads"

/** The optional parts of a backup, as `GET /api/backup` takes them. */
export type BackupParts = {
  images: boolean
  volumes: boolean
  files: boolean
  secrets: boolean
}

/** A backup's first entry: what it holds, by name, without a secret. */
export type BackupManifest = {
  kind: string
  format: number
  version: string
  created_at: number
  included: BackupParts
  containers: { id: string; name: string; files: boolean }[]
  images: { id: string; name: string; image: boolean }[]
  volumes: { name: string; included: boolean; owner?: string; parameter?: string }[]
  endpoints: string[]
  scale_sets: string[]
  parts: { kind: "image" | "files" | "volume"; target: string }[]
  secrets_left_out: string[]
  scale_sets_secrets_left_out: string[]
}

export type BackupConflict = {
  kind: "container" | "image" | "endpoint" | "volume" | "scale-set"
  name: string
}

/** What `POST /api/backup/check` says a restore would meet here. */
export type BackupReview = { conflicts: BackupConflict[]; missing_volumes: string[] }

export const MANIFEST = "thoric-backup.json"
const KIND = "thoric-backup"
/** The manifest layout this release reads; the manager refuses any other one too. */
export const BACKUP_FORMAT = 1
const BLOCK = 512
/** As the manager reads it. */
const MANIFEST_LIMIT = 4 << 20

/** The download of a backup with `parts`. */
export function backupUrl(parts: BackupParts) {
  const query = new URLSearchParams(
    Object.entries(parts).map(([part, on]) => [part, on ? "true" : "false"])
  )
  return `/api/backup?${query}`
}

/** The length of the manifest entry an archive opens with, or 0 when it opens with another. */
function manifestLength(tar: Uint8Array) {
  if (tar.length < BLOCK) return 0
  const field = (start: number, length: number) => {
    const bytes = tar.subarray(start, start + length)
    const end = bytes.indexOf(0)
    return new TextDecoder().decode(end < 0 ? bytes : bytes.subarray(0, end))
  }
  if (field(0, 100) !== MANIFEST || tar[156] !== 0x30) return 0
  const size = parseInt(field(124, 12).trim(), 8)
  return size > 0 && size <= MANIFEST_LIMIT ? BLOCK + size : 0
}

/** The manifest at the start of an unpacked backup; null for any other file. */
export function parseManifest(tar: Uint8Array): BackupManifest | null {
  const length = manifestLength(tar)
  if (!length || tar.length < length) return null
  try {
    const manifest = JSON.parse(new TextDecoder().decode(tar.subarray(BLOCK, length)))
    return manifest?.kind === KIND &&
      typeof manifest.format === "number" &&
      Array.isArray(manifest.containers) &&
      Array.isArray(manifest.images) &&
      Array.isArray(manifest.volumes) &&
      Array.isArray(manifest.endpoints) &&
      Array.isArray(manifest.parts)
      ? ({
          secrets_left_out: [],
          scale_sets: [],
          scale_sets_secrets_left_out: [],
          ...manifest,
        } as BackupManifest)
      : null
  } catch {
    return null
  }
}

/** Reads the manifest from the start of a backup file, without reading the rest of it. */
export async function readManifest(file: Blob): Promise<BackupManifest | null> {
  const head = new Uint8Array(await file.slice(0, MANIFEST_LIMIT + 2 * BLOCK).arrayBuffer())
  if (head[0] !== 0x1f || head[1] !== 0x8b) return parseManifest(head)
  const reader = new Blob([head]).stream().pipeThrough(new DecompressionStream("gzip")).getReader()
  const chunks: Uint8Array[] = []
  let length = 0
  let wanted = BLOCK
  try {
    while (length < wanted) {
      const { done, value } = await reader.read()
      if (done) break
      chunks.push(value)
      length += value.length
      // Once the first header is in, read on only as far as the manifest reaches.
      if (wanted === BLOCK && length >= BLOCK) {
        wanted = manifestLength(join(chunks, length))
        if (!wanted) break
      }
    }
  } catch {
    // The slice ends inside the compressed stream; the manifest, if any, was read before that.
  }
  reader.cancel().catch(() => {})
  return parseManifest(join(chunks, length))
}

function join(chunks: Uint8Array[], length: number) {
  const out = new Uint8Array(length)
  let offset = 0
  for (const chunk of chunks) {
    out.set(chunk, offset)
    offset += chunk.length
  }
  return out
}

/** What a restore of `manifest` creates, by name, as its confirmation lists it. */
export function restorePlan(manifest: BackupManifest) {
  const names = (items: { name: string }[]) => items.map((item) => item.name)
  const left = new Set(manifest.secrets_left_out)
  return {
    containers: names(manifest.containers.filter((container) => container.files)),
    withoutFiles: names(manifest.containers.filter((container) => !container.files)),
    images: names(manifest.images.filter((image) => image.image)),
    recordsOnly: names(manifest.images.filter((image) => !image.image)),
    volumes: names(manifest.volumes.filter((volume) => volume.included)),
    endpoints: manifest.endpoints,
    scaleSets: manifest.scale_sets,
    secretsLeftOut: names(manifest.containers.filter((container) => left.has(container.id))),
  }
}

/** Whether this release can restore `manifest`. */
export const readable = (manifest: BackupManifest) => manifest.format === BACKUP_FORMAT

let upload: ArchiveImport | null = null
let version = 0
const listeners = new Set<() => void>()

function changed() {
  version += 1
  for (const listener of listeners) listener()
}

/**
 * The restore upload, kept outside the page so that it goes on while the dashboard shows another
 * page; leaving or reloading Thoric ends it.
 */
export function restoreUpload(
  createRequest: () => UploadRequest = () => new XMLHttpRequest(),
  onUnauthorized: () => void = sessionExpired
) {
  upload ??= new ArchiveImport({
    path: "backup/restore",
    createRequest,
    onChange: changed,
    onUnauthorized,
  })
  return upload
}

export function subscribeRestore(listener: () => void) {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export const restoreVersion = () => version
