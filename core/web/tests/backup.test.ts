/* eslint-disable @typescript-eslint/no-explicit-any -- the fake stands in for XMLHttpRequest */
import { test } from "bun:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { gzipSync } from "node:zlib"

import {
  BACKUP_FORMAT,
  backupUrl,
  parseManifest,
  readable,
  readManifest,
  restorePlan,
  restoreUpload,
  subscribeRestore,
  restoreVersion,
  type BackupManifest,
} from "../src/lib/backup"
import { jobTarget } from "../src/lib/format"

/** One USTAR entry as the manager writes it, padded to whole blocks. */
function entry(name: string, data: Uint8Array) {
  const header = new Uint8Array(512)
  const put = (offset: number, text: string) => header.set(new TextEncoder().encode(text), offset)
  const octal = (offset: number, width: number, value: number) =>
    put(offset, value.toString(8).padStart(width - 1, "0") + "\0")
  put(0, name)
  octal(100, 8, 0o644)
  octal(108, 8, 0)
  octal(116, 8, 0)
  octal(124, 12, data.length)
  octal(136, 12, 0)
  put(156, "0")
  put(257, "ustar\0")
  put(263, "00")
  put(148, "        ")
  const sum = header.reduce((total, byte) => total + byte, 0)
  put(148, sum.toString(8).padStart(6, "0") + "\0 ")
  const out = new Uint8Array(512 + Math.ceil(data.length / 512) * 512)
  out.set(header)
  out.set(data, 512)
  return out
}

const json = (value: unknown) => new TextEncoder().encode(JSON.stringify(value))

const manifest: BackupManifest = {
  kind: "thoric-backup",
  format: 1,
  version: "0.7.0",
  created_at: 1_792_676_706,
  included: { images: true, volumes: true, files: true, secrets: false },
  containers: [
    { id: "c1", name: "Unity", files: true },
    { id: "c2", name: "Scratch", files: false },
  ],
  images: [
    { id: "i1", name: "Unity tools", image: true },
    { id: "i2", name: "Lost build", image: false },
  ],
  volumes: [
    { name: "projects", included: true },
    { name: "elsewhere", included: false },
  ],
  endpoints: ["unity", "re"],
  scale_sets: ["re"],
  parts: [
    { kind: "image", target: "i1" },
    { kind: "files", target: "c1" },
    { kind: "volume", target: "projects" },
  ],
  secrets_left_out: ["c1"],
  scale_sets_secrets_left_out: ["re"],
}

/** A backup as the manager streams one: the manifest, then records and a part that does not compress. */
function backup(first = entry("thoric-backup.json", json(manifest))) {
  let seed = 2463534242
  const part = new Uint8Array(3 << 20).map(() => {
    seed ^= seed << 13
    seed ^= seed >>> 17
    seed ^= seed << 5
    return seed & 0xff
  })
  const entries = [first, entry("records.json", json({})), entry("parts/0000/00000000", part)]
  const out = new Uint8Array(entries.reduce((total, item) => total + item.length, 0))
  let offset = 0
  for (const item of entries) {
    out.set(item, offset)
    offset += item.length
  }
  return out
}

test("a backup's download names each optional part", () => {
  assert.equal(
    backupUrl({ images: true, volumes: false, files: true, secrets: false }),
    "/api/backup?images=true&volumes=false&files=true&secrets=false"
  )
})

test("the manifest is read from the start of a gzip or plain backup", async () => {
  const tar = backup()
  const gzip = gzipSync(tar, { level: 1 })
  assert.ok(gzip.length > 1 << 20, "the start is only a slice of the file")
  assert.deepEqual(await readManifest(new Blob([gzip])), manifest)
  assert.deepEqual(await readManifest(new Blob([tar])), manifest)
  assert.equal(await readManifest(new Blob([new Uint8Array(2048)])), null)
  assert.equal(await readManifest(new Blob([gzip.subarray(0, 40)])), null)
})

test("only a backup's manifest is taken for one", async () => {
  // An image archive opens with its own metadata, not a backup's manifest.
  const image = backup(entry("thoric.json", json({ format: 1, name: "Unity" })))
  assert.equal(await readManifest(new Blob([gzipSync(image)])), null)
  assert.equal(parseManifest(entry("thoric-backup.json", json({ ...manifest, kind: "x" }))), null)
  assert.equal(parseManifest(entry("thoric-backup.json", new TextEncoder().encode("{"))), null)
  // A manifest without the list of left-out secrets reads as one that left none out.
  const older: Partial<BackupManifest> = { ...manifest }
  delete older.secrets_left_out
  assert.deepEqual(parseManifest(entry("thoric-backup.json", json(older)))?.secrets_left_out, [])
  // One from a release before scale sets reads as one that holds none.
  const beforeSets: Partial<BackupManifest> = { ...manifest }
  delete beforeSets.scale_sets
  delete beforeSets.scale_sets_secrets_left_out
  const read = parseManifest(entry("thoric-backup.json", json(beforeSets)))
  assert.deepEqual([read?.scale_sets, read?.scale_sets_secrets_left_out], [[], []])
  assert.ok(readable(manifest))
  assert.ok(!readable({ ...manifest, format: BACKUP_FORMAT + 1 }))
})

test("the confirmation names what a restore creates, and what it creates without", () => {
  assert.deepEqual(restorePlan(manifest), {
    containers: ["Unity"],
    withoutFiles: ["Scratch"],
    images: ["Unity tools"],
    recordsOnly: ["Lost build"],
    volumes: ["projects"],
    endpoints: ["unity", "re"],
    scaleSets: ["re"],
    secretsLeftOut: ["Unity"],
  })
})

test("the restore upload goes to the restore route and outlives the page that started it", async () => {
  const requests: any[] = []
  const upload = restoreUpload(() => {
    const request: any = {
      upload: {},
      headers: {},
      open(_method: string, url: string) {
        request.url = new URL(url, "http://localhost")
      },
      setRequestHeader(key: string, value: string) {
        request.headers[key] = value
      },
      send() {},
      abort() {
        request.onabort()
      },
      complete(status: number, result: unknown) {
        request.status = status
        request.responseText = JSON.stringify(result)
        request.onload()
      },
    }
    requests.push(request)
    return request
  })
  // Asked for again, from another page, it is the same upload.
  assert.equal(restoreUpload(), upload)
  let changes = 0
  const stop = subscribeRestore(() => changes++)
  const before = restoreVersion()
  const done = upload.start({ size: 10 } as any as Blob, {}, 100)
  const request = requests[0]
  assert.equal(request.url.pathname, "/api/backup/restore")
  assert.deepEqual(Object.fromEntries(request.url.searchParams), { size: "10" })
  request.complete(200, { job_id: "job-7" })
  assert.equal(await done, "job-7")
  assert.ok(changes > 0 && restoreVersion() > before)
  stop()
})

test("a backup or a restore job names the installation as what it worked on", () => {
  assert.equal(jobTarget({ resource: "installation" }, new Map()), "Thoric")
})

// The routes this page calls are the manager's; a route renamed there fails here too.
test("the manager serves the backup routes the page calls", () => {
  const router = readFileSync(new URL("../../manager/src/main.rs", import.meta.url), "utf8")
  for (const route of ['"/api/backup"', '"/api/backup/check"', '"/api/backup/restore"'])
    assert.ok(router.includes(route), `core/manager/src/main.rs must route ${route}`)
})
