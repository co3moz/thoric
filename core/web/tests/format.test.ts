import { test } from "bun:test"
import { readFileSync } from "node:fs"
import assert from "node:assert/strict"

import {
  deletableImage,
  deletedNotice,
  downloadProgress,
  containerStatus,
  desktopTitle,
  duration,
  healthReason,
  imageChoices,
  imageHolders,
  imageSource,
  imageStatus,
  imageTree,
  jobActivity,
  jobLogText,
  jobProgress,
  jobTarget,
  needsAttention,
  recentJobs,
  restartNotice,
  scriptParts,
} from "../src/lib/format"
import type { ContainerItem, ContainerSetup, ImageItem, JobStatus } from "../src/lib/types"

test("completed legacy deletion explains the result rather than waiting or displaying an ID", () => {
  const log = jobLogText({
    status: "completed",
    log: "",
    result: { deleted: "old-container-id", volumes_preserved: true },
  })
  assert.equal(log, "Container deleted. Named volumes were preserved.")
  assert.equal(jobLogText({ status: "queued", log: "" }), "Waiting to start…")
  assert.equal(
    jobLogText({ status: "failed", log: "" }),
    "Operation failed. No detailed log was recorded."
  )
  assert.equal(
    jobLogText({ status: "failed", log: "Ownership check failed." }),
    "Ownership check failed."
  )
})

test("deep snapshot history cannot overflow the JavaScript call stack", () => {
  const records = Array.from({ length: 20000 }, (_, i) => ({
    id: `r${i}`,
    image_id: `image${i}`,
    parent_image_id: i ? `image${i - 1}` : null,
  }))
  const tree = imageTree(records)
  assert.equal(tree.length, 20000)
  assert.equal(tree.at(-1)!.depth, 19999)
})

test("image branches follow immutable parent image IDs, not mutable tags", () => {
  const records = [
    { id: "b", image_id: "sha:b", parent_image_id: "sha:a" },
    { id: "c", image_id: "sha:c", parent_image_id: "sha:a" },
    { id: "a", image_id: "sha:a", parent_image_id: "sha:base" },
  ]
  assert.deepEqual(
    imageTree(records).map((r) => [r.id, r.depth]),
    [
      ["a", 0],
      ["b", 1],
      ["c", 1],
    ]
  )
})

test("missing parents, duplicate image IDs and cycles remain visible", () => {
  const records = [
    { id: "a", image_id: "sha:a", parent_image_id: "sha:b" },
    { id: "b", image_id: "sha:b", parent_image_id: "sha:a" },
    { id: "c", image_id: "sha:a", parent_image_id: null },
  ]
  assert.equal(imageTree(records).length, 3)
  assert.equal(new Set(imageTree(records).map((r) => r.id)).size, 3)
})

const image = (fields: Partial<ImageItem>): ImageItem => ({
  id: "i",
  name: "Image",
  image_id: null,
  reference: "",
  parent_image_id: null,
  source_container: null,
  kind: "snapshot",
  created_at: null,
  description: "",
  ...fields,
})
const catalogImage = (reference: string, name: string, fields: Partial<ImageItem> = {}) =>
  image({ id: reference, reference, name, kind: "catalog", available: false, ...fields })

test("image choices distinguish GNOME, XFCE and unavailable images", () => {
  const images = [
    catalogImage("xfce:24.04", "Ubuntu · Lightweight XFCE", {
      available: true,
      image_id: "sha256:x",
    }),
    catalogImage("gnome:24.04", "Ubuntu · GNOME"),
    catalogImage("other:24.04", "Ubuntu · Other", {
      image_id: "sha256:wrong",
      downloaded: true,
      error: "Image labels do not match the catalog",
    }),
    image({
      image_id: "sha256:custom",
      name: "My game",
      available: true,
      desktop: { catalog_id: "ubuntu-gnome", title: "Ubuntu · GNOME", session: "gnome" },
    }),
  ]
  const choices = imageChoices(images, "xfce:24.04")
  assert.equal(choices.length, 4)
  // A catalog image that is not downloaded stays selectable: creating from it downloads it first.
  assert.deepEqual(
    [choices[1].disabled, choices[1].label],
    [false, "Ubuntu · GNOME · downloads first"]
  )
  assert.deepEqual([choices[2].disabled, choices[2].label], [true, "Ubuntu · Other"])
  assert.equal(choices[3].label, "My game · Ubuntu · GNOME")
  // The select marks catalog images with a cloud and saved ones with the image icon.
  assert.deepEqual(
    choices.map((choice) => choice.catalog),
    [true, true, true, false]
  )
})

test("catalog images are images: listed, started and nested like the others", () => {
  const xfce = catalogImage("xfce:24.04", "XFCE", { available: true, image_id: "sha:x" })
  const gnome = catalogImage("gnome:24.04", "GNOME")
  const snapshot = image({ id: "s", image_id: "sha:s", parent_image_id: "sha:x" })
  // Children of a downloaded catalog image sit under it; one without a Docker ID has none.
  assert.deepEqual(
    imageTree([xfce, gnome, snapshot]).map((r) => [r.id, r.depth]),
    [
      ["xfce:24.04", 0],
      ["s", 1],
      ["gnome:24.04", 0],
    ]
  )
  // A catalog image starts by its reference, so a missing one downloads first.
  assert.equal(imageSource(gnome), "gnome:24.04")
  assert.equal(imageSource(snapshot), "sha:s")
  assert.equal(imageStatus(gnome).key, "not_ready")
  assert.equal(imageStatus({ ...gnome, error: "Image labels do not match" }).key, "unusable")
  assert.equal(imageStatus({ ...snapshot, available: false }).key, "missing")
  assert.equal(imageStatus({ ...snapshot, available: null }).key, "unknown")
  const spec = { image: "gnome:24.04" } as ContainerItem["spec"]
  assert.equal(desktopTitle(spec, undefined, [xfce, gnome]), "GNOME")
  assert.equal(desktopTitle(spec, "Ubuntu · GNOME", [xfce, gnome]), "Ubuntu · GNOME")
})

test("download progress reads as bytes once the size is known and never claims a size it lacks", () => {
  assert.equal(downloadProgress(null), null)
  assert.deepEqual(downloadProgress({ current: 0, total: 0, layers: 0, layers_done: 0 }), {
    text: "Preparing download…",
  })
  // Until every remaining layer reports its size there is no honest bar, only the count so far.
  assert.deepEqual(
    downloadProgress({ current: 3 * 1024 ** 2, total: null, layers: 4, layers_done: 1 }),
    {
      text: "3.0 MiB downloaded · 1 of 4 layers",
    }
  )
  assert.deepEqual(downloadProgress({ current: 0, total: 0, layers: 2, layers_done: 2 }), {
    text: "Checking layers · 2 of 2 layers",
  })
  const running = downloadProgress({
    current: 1024 ** 3,
    total: 4 * 1024 ** 3,
    layers: 4,
    layers_done: 1,
  })
  assert.deepEqual(running, {
    value: 1024 ** 3,
    max: 4 * 1024 ** 3,
    percent: 25,
    text: "1.0 GiB of 4.0 GiB · 1 of 4 layers",
  })
  assert.equal(
    downloadProgress({ current: 10, total: 10, layers: 4, layers_done: 3 })!.text,
    "Extracting · 3 of 4 layers"
  )
})

test("the restart warning names the settings that recreate the container and what happens", () => {
  assert.equal(
    restartNotice(["audio"], true),
    "Audio recreates the container: it restarts and open applications close. Its files are kept."
  )
  assert.equal(
    restartNotice(["gpu", "mounts", "cpus"], false),
    "GPU, volumes and removing the CPU limit recreate the container: it stays stopped until you start it. Its files are kept."
  )
  assert.equal(
    restartNotice(["hostname", "mac_address", "machine_id"], true),
    "Hostname, MAC address and machine ID recreate the container: it restarts and open applications close. Its files are kept."
  )
})

test("job activity sums the hourly counts per local day, with or without scripts", () => {
  const now = new Date(2026, 9, 14, 12)
  const at = (days: number, hour = 10) => new Date(2026, 9, 14 - days, hour).getTime() / 1000
  const count = (
    cls: "script" | "other",
    at: number,
    completed: number,
    failed = 0,
    interrupted = 0
  ) => ({ at, class: cls, completed, failed, interrupted })
  const hours = [
    count("other", at(0), 1, 1),
    count("script", at(0), 40, 2),
    count("other", at(1), 0, 0, 1),
    // The last hour of yesterday and the first of today are two days.
    count("script", at(1, 23), 5),
    count("other", at(0, 0), 1),
    count("other", at(6), 2),
    // Before the range, and after it.
    count("other", at(7), 9),
    count("other", at(-1), 9),
  ]
  const points = jobActivity(hours, 7, true, now)
  assert.equal(points.length, 7)
  assert.equal(points[0].date, "2026-10-08")
  assert.deepEqual(points[0], { date: "2026-10-08", completed: 2, failed: 0 })
  assert.deepEqual(points.at(-1), { date: "2026-10-14", completed: 42, failed: 3 })
  // An interrupted job did not finish as asked either.
  assert.deepEqual(points.at(-2), { date: "2026-10-13", completed: 5, failed: 1 })
  const others = jobActivity(hours, 7, false, now)
  assert.deepEqual(others.at(-1), { date: "2026-10-14", completed: 2, failed: 1 })
  assert.deepEqual(others.at(-2), { date: "2026-10-13", completed: 0, failed: 1 })
  // A range the counts do not reach is days without jobs, never a shorter chart.
  assert.equal(jobActivity([], 30, true, now).length, 30)
})

test("the jobs menu lists jobs in progress before newer finished ones", () => {
  const job = (id: string, status: JobStatus, created_at: number) => ({ id, status, created_at })
  const jobs = [
    job("old-done", "completed", 1),
    job("old-run", "running", 2),
    job("failed", "failed", 5),
    job("queued", "queued", 3),
    job("new-done", "completed", 6),
  ]
  const ids = (limit: number) => recentJobs(jobs, limit).map((entry) => entry.id)
  assert.deepEqual(ids(10), ["queued", "old-run", "new-done", "failed", "old-done"])
  assert.deepEqual(ids(3), ["queued", "old-run", "new-done"])
  // The list it was given keeps its order.
  assert.equal(jobs[0].id, "old-done")
})

test("a job names what it worked on, even after that container was deleted", () => {
  const containers = new Map([["e1", "Unity workspace"]])
  const images = new Map([
    ["ubuntu-gnome", "Ubuntu 24.04 · GNOME"],
    ["i1", "Unity snapshot"],
  ])
  const target = (resource: string | null, result?: Record<string, unknown>) =>
    jobTarget({ resource, result }, containers, images)
  assert.equal(target("container:e1"), "Unity workspace")
  assert.equal(target("container:gone", { deleted: "gone", name: "Old QA" }), "Old QA")
  assert.equal(target("container:0123456789abcdef"), "Container 01234567")
  // Jobs recorded before the rename still find their container.
  assert.equal(target("environment:e1"), "Unity workspace")
  // A catalog image's jobs name it by its catalog ID; older ones called it a profile.
  assert.equal(target("image:ubuntu-gnome"), "Ubuntu 24.04 · GNOME")
  assert.equal(target("profile:ubuntu-gnome"), "Ubuntu 24.04 · GNOME")
  assert.equal(target("profile:retired"), "retired")
  assert.equal(target(null), "")
  assert.equal(target("image:i1"), "Unity snapshot")
  assert.equal(target("image:new", { name: "Imported" }), "Imported")
  assert.equal(target("image:0123456789ab"), "Image 01234567")
})

test("archive jobs count bytes; image downloads keep counting layers", () => {
  assert.deepEqual(jobProgress({ transfer: "export", current: 0 }), {
    title: "Archive progress",
    text: "Docker is preparing the image…",
  })
  assert.deepEqual(jobProgress({ transfer: "export", current: 3 * 1024 ** 3 }), {
    title: "Archive progress",
    text: "3.0 GiB exported",
  })
  assert.deepEqual(jobProgress({ transfer: "import", current: 1024 ** 3, total: 4 * 1024 ** 3 }), {
    title: "Archive progress",
    value: 1024 ** 3,
    max: 4 * 1024 ** 3,
    percent: 25,
    text: "1.0 GiB of 4.0 GiB received",
  })
  // An archive streamed without a size has no total: bytes, and no percentage to show.
  assert.deepEqual(jobProgress({ transfer: "import", current: 3 * 1024 ** 3 }), {
    title: "Archive progress",
    text: "3.0 GiB received",
  })
  // A volume copy counts the bytes it has copied; nothing was received.
  assert.deepEqual(jobProgress({ transfer: "copy", current: 2 * 1024 ** 2 }), {
    title: "Archive progress",
    text: "2.0 MiB copied",
  })
  assert.deepEqual(jobProgress({ current: 0, total: 0, layers: 0, layers_done: 0 }), {
    ...downloadProgress({ current: 0, total: 0, layers: 0, layers_done: 0 }),
    title: "Image download progress",
  })
  assert.equal(jobProgress(null), null)
})

test("a script's progress names its part, then the step it named or the line it reached", () => {
  const title = "Setup script progress"
  assert.deepEqual(
    jobProgress({
      script: {
        part: 2,
        parts: 3,
        name: "Ghidra",
        stage: 3,
        stages: 10,
        label: "Installing Ghidra",
        percent: 47,
      },
    }),
    {
      title,
      value: 47,
      max: 100,
      percent: 47,
      text: "Ghidra, part 2 of 3 · Step 3 of 10: Installing Ghidra",
    }
  )
  assert.equal(
    jobProgress({
      script: { part: 1, parts: 1, stage: 4, stages: 5, label: "Wine", skipped: true, percent: 80 },
    })?.text,
    "Step 4 of 5 skipped: Wine"
  )
  assert.equal(
    jobProgress({
      script: {
        part: 1,
        parts: 1,
        line: 42,
        lines: 120,
        label: "apt-get install -y gdb",
        percent: 34,
      },
    })?.text,
    "Line 42 of 120: apt-get install -y gdb"
  )
  // A part that has said nothing yet is still running.
  assert.deepEqual(jobProgress({ script: { part: 1, parts: 1, percent: 0 } }), {
    title,
    value: 0,
    max: 100,
    percent: 0,
    text: "Running the script",
  })
})

// A running stage counts up to the current time, which has a fraction.
test("a time just short of a minute reads as the next minute", () => {
  assert.equal(duration(59.6), "1m 0s")
  assert.equal(duration(479.6), "8m 0s")
  assert.equal(duration(65.2), "1m 5s")
  assert.equal(duration(-1), "0s")
})

test("a script's steps show what was skipped, what is to come and where a job stopped", () => {
  const steps = (job: Parameters<typeof scriptParts>[0]) =>
    scriptParts(job, 100).map((part) => ({
      ...part,
      steps: part.steps.map(({ number, label, state, seconds }) => [number, label, state, seconds]),
    }))
  const running = steps({
    status: "running",
    finished_at: null,
    script: {
      parts: [
        { name: "Base", started_at: 10, finished_at: 10, cached: true },
        {
          name: "Ghidra",
          started_at: 20,
          stages: 10,
          steps: [
            { stage: 1, label: "Download", started_at: 20, finished_at: 40 },
            { stage: 2, label: "Wine", skipped: true },
            { stage: 5, started_at: 40 },
          ],
        },
        { name: "GDB" },
      ],
    },
  })
  assert.deepEqual(
    running.map(({ name, state, seconds, cached }) => [name, state, seconds, cached]),
    [
      ["Base", "done", 0, true],
      ["Ghidra", "running", 80, false],
      ["GDB", "pending", null, false],
    ]
  )
  assert.deepEqual(running[1].steps, [
    ["1/10", "Download", "done", 20],
    ["2/10", "Wine", "skipped", null],
    [undefined, "Steps 3-4", "skipped", null],
    ["5/10", "Step 5", "running", 60],
    [undefined, "5 more steps", "pending", null],
  ])
  // A part that ended well skipped the steps it never named; the one a failed job stopped in
  // did not reach them.
  const ended = steps({
    status: "failed",
    finished_at: 90,
    script: {
      parts: [
        {
          name: "A",
          started_at: 10,
          finished_at: 20,
          stages: 3,
          steps: [{ stage: 1, started_at: 10 }],
        },
        { name: "B", started_at: 20, stages: 3, steps: [{ stage: 2, started_at: 30 }] },
      ],
    },
  })
  assert.deepEqual(ended[0].steps, [
    ["1/3", "Step 1", "done", 10],
    [undefined, "Steps 2-3", "skipped", null],
  ])
  assert.equal(ended[1].state, "failed")
  assert.deepEqual(ended[1].steps, [
    [undefined, "Step 1", "skipped", null],
    ["2/3", "Step 2", "failed", 60],
    [undefined, "1 step not reached", "pending", null],
  ])
  assert.deepEqual(scriptParts({ status: "completed", finished_at: 5, script: null }, 100), [])
})

// The manager writes the progress and the report the interface reads; a field renamed there
// would leave the panel without its steps.
test("the manager writes every script field the interface reads", () => {
  const source = readFileSync(
    new URL("../../manager/src/script_progress.rs", import.meta.url),
    "utf8"
  )
  for (const key of [
    "part",
    "parts",
    "name",
    "stage",
    "stages",
    "label",
    "skipped",
    "line",
    "lines",
    "percent",
  ])
    assert.ok(
      source.includes(`"${key}"`) || source.includes(`summary["${key}"]`),
      `progress ${key}`
    )
  for (const field of [
    "parts",
    "name",
    "started_at",
    "finished_at",
    "cached",
    "stages",
    "steps",
    "stage",
    "label",
    "skipped",
  ])
    assert.ok(new RegExp(String.raw`pub ${field}:`).test(source), `report ${field}`)
  assert.ok(source.includes(`json!({ "script": summary })`))
})

test("a running container shows its compose setup until the setup completes", () => {
  const item = (status?: ContainerSetup["status"], last_error?: string): ContainerItem => ({
    id: "e",
    container: "thoric-e",
    source_image_id: "sha256:a",
    created_at: 0,
    spec: {} as ContainerItem["spec"],
    last_error,
    setup: status ? { status, image: "i", parameters: {}, secrets: [] } : null,
    state: "running",
    health: "healthy",
    status: "Up",
  })
  assert.equal(containerStatus(item()).key, "running")
  assert.equal(containerStatus(item("pending")).key, "setting_up")
  assert.equal(containerStatus(item("running")).key, "setting_up")
  assert.equal(containerStatus(item("completed")).key, "running")
  assert.equal(containerStatus(item("failed", "The compose setup failed")).key, "setup_failed")
  // A setup that never started because recreating the container failed is not in progress.
  assert.equal(containerStatus(item("pending", "Docker: no such image")).key, "setup_failed")
  assert.equal(needsAttention(item("failed")), true)
  assert.equal(needsAttention(item("completed")), false)
  // Until the desktop answers its health check, it is starting, whatever the setup says.
  assert.equal(containerStatus({ ...item("pending"), health: "starting" }).key, "created")
})

test("an unhealthy desktop says why in words, and a reason it does not know as the check said it", () => {
  const unhealthy = (health_detail?: string | null) => ({
    health: "unhealthy" as const,
    health_detail,
  })
  assert.match(
    healthReason(unhealthy("display-held: no answer in 3 s")) ?? "",
    /a program holds it/
  )
  assert.match(
    healthReason(unhealthy("Health check exceeded timeout (4s)")) ?? "",
    /did not finish in time/
  )
  assert.equal(healthReason(unhealthy("Xvfb crashed")), "Xvfb crashed")
  assert.equal(healthReason(unhealthy(null)), null)
  assert.equal(healthReason({ health: "healthy", health_detail: "display-held: old" }), null)
})

test("every failure the desktop's health check names has words in the dashboard", () => {
  const script = readFileSync(
    new URL("../../../images/desktop/desktop-healthcheck.sh", import.meta.url),
    "utf8"
  )
  const words = [...script.matchAll(/echo '([a-z-]+): /g)].map((match) => match[1])
  assert.deepEqual([...new Set(words)].sort(), [
    "desktop-starting",
    "display-gone",
    "display-held",
    "session-failed",
  ])
  for (const word of words) {
    const reason = healthReason({ health: "unhealthy", health_detail: `${word}: detail` })
    assert.notEqual(reason, `${word}: detail`, word)
  }
})

test("an image offers Delete only with something to delete, and names who holds it", () => {
  const xfce = catalogImage("xfce:24.04", "XFCE", {
    image_id: "sha:x",
    downloaded: true,
    available: true,
  })
  const gnome = catalogImage("gnome:24.04", "GNOME")
  const build = image({ id: "b", kind: "build", image_id: "sha:b", available: true })
  const host = image({ id: "h", kind: "import", image_id: "sha:h", available: true })
  // A catalog image that is not downloaded has no download to delete.
  assert.equal(deletableImage(xfce), true)
  assert.equal(deletableImage(gnome), false)
  assert.equal(deletableImage(build), true)
  const containers = [
    { spec: { name: "Editor" }, source_image_id: "sha:b" },
    { spec: { name: "Runner" }, source_image_id: "sha:b" },
    { spec: { name: "Plain" }, source_image_id: "sha:x" },
    { spec: { name: "Hosted" }, source_image_id: "sha:h" },
  ] as Parameters<typeof imageHolders>[1]
  const images = [xfce, gnome, build, host]
  assert.deepEqual(imageHolders(build, containers, images), ["Editor", "Runner"])
  assert.deepEqual(imageHolders(xfce, containers, images), ["Plain"])
  assert.deepEqual(imageHolders(gnome, containers, images), [])
  // An import keeps its Docker image, so a container from it does not hold its record.
  assert.deepEqual(imageHolders(host, containers, images), [])
  // An archive of the build imported here keeps the same image, so either record can go.
  const copy = image({ id: "c", kind: "archive", image_id: "sha:b", available: true })
  assert.deepEqual(imageHolders(build, containers, [...images, copy]), [])
  assert.deepEqual(imageHolders(copy, containers, [...images, copy]), [])
  // A build sharing the catalog image's ID does not keep the catalog download.
  const same = image({ id: "same", kind: "build", image_id: "sha:x" })
  assert.deepEqual(imageHolders(xfce, containers, [...images, same]), ["Plain"])
  // Docker keeps an image another tag or image still holds; the notice says so only then.
  assert.equal(deletedNotice(build, true), "images.deleted")
  assert.equal(deletedNotice(build, false), "images.deletedKept")
  assert.equal(deletedNotice(xfce, false), "images.deletedKept")
  assert.equal(deletedNotice(host, false), "images.deleted")
  assert.equal(deletedNotice({ ...build, available: false }, false), "images.deleted")
})
