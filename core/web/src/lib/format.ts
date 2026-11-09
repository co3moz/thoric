import {
  formatDate,
  formatList,
  formatNumber,
  getLocale,
  has,
  type MessageKey,
  plural,
  t,
} from "./i18n"
import { iconUrl } from "./icons"
import { fileSize } from "./uploads"
import type {
  Container,
  ContainerItem,
  ContainerSpec,
  DownloadProgress,
  ImageItem,
  Job,
  JobCount,
  ScriptSummary,
} from "./types"

const oneDecimal = { minimumFractionDigits: 1, maximumFractionDigits: 1 }

export const memory = (bytes?: number | null) =>
  bytes
    ? t("units.gigabytes", { value: formatNumber(bytes / 1024 ** 3, oneDecimal) })
    : t("common.unknown")
export const date = (seconds: number) => formatDate(seconds)
export const shortDate = (seconds: number) =>
  formatDate(seconds, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  })

export function duration(time: number) {
  // Rounded first, so 59.6 seconds past a minute reads as the next minute and never as 60 seconds.
  const seconds = Math.max(0, Math.round(time))
  if (seconds < 60) return t("duration.seconds", { seconds })
  if (seconds < 3600)
    return t("duration.minutes", {
      minutes: Math.floor(seconds / 60),
      seconds: seconds % 60,
    })
  return t("duration.hours", {
    hours: Math.floor(seconds / 3600),
    minutes: Math.floor((seconds % 3600) / 60),
  })
}

/** A container, image or job state in words; an unknown one as Docker named it. */
export function stateLabel(state: string) {
  const key = `state.${state}`
  return has(key) ? t(key) : state
}

// The kinds these jobs had before catalog images were listed with the others.
const renamedJobs: Record<string, string> = {
  "profile-pull": "image-pull",
  "profile-build": "source-build",
}

/** The name of a job kind; one this release does not know as the manager recorded it. */
export function jobName(kind: string) {
  const key = `job.${renamedJobs[kind] ?? kind}`
  return has(key) ? t(key) : kind
}

export type Tone = "success" | "warning" | "danger" | "muted" | "info"

/** What a card or row shows as the container's state, and the color family for it. */
export function containerStatus(item: ContainerItem): { key: string; tone: Tone } {
  const running = item.state === "running"
  if (item.health === "unhealthy") return { key: "unhealthy", tone: "danger" }
  if (running && item.health === "starting") return { key: "created", tone: "warning" }
  // A compose setup that is waiting with an error recorded did not get to run.
  const setup = item.setup?.status
  const failed = setup === "failed" || (setup === "pending" && !!item.last_error)
  if (running && failed) return { key: "setup_failed", tone: "danger" }
  if (running && (setup === "pending" || setup === "running"))
    return { key: "setting_up", tone: "info" }
  if (running) return { key: "running", tone: "success" }
  if (item.state === "missing" || item.state === "unavailable" || item.last_error)
    return { key: item.state, tone: "danger" }
  if (["created", "restarting", "paused"].includes(item.state))
    return { key: item.state, tone: "warning" }
  return { key: item.state, tone: "muted" }
}

// What a desktop's health check begins a failure with, and Docker's words for one that did not
// finish, as images before the words had it.
const healthReasons: [string, MessageKey][] = [
  ["display-held:", "health.displayHeld"],
  ["display-gone:", "health.displayGone"],
  ["desktop-starting:", "health.starting"],
  ["session-failed:", "health.sessionFailed"],
  ["Health check exceeded timeout", "health.timedOut"],
]

/** Why an unhealthy desktop is, in words; a reason this release does not know as the check said it. */
export function healthReason(item: Pick<ContainerItem, "health" | "health_detail">) {
  const detail = item.health === "unhealthy" ? item.health_detail?.trim() : null
  if (!detail) return null
  const known = healthReasons.find(([start]) => detail.startsWith(start))
  return known ? t(known[1]) : detail
}

/** A stopped container whose setup did not finish offers a retry instead of a start. */
export const needsRetry = (item: ContainerItem) =>
  item.state !== "running" && (item.state === "missing" || !!item.last_error)

export const needsAttention = (item: ContainerItem) =>
  needsRetry(item) ||
  item.health === "unhealthy" ||
  item.state === "unavailable" ||
  containerStatus(item).key === "setup_failed"

export function jobTone(status: string): Tone {
  if (status === "completed") return "success"
  if (status === "failed" || status === "interrupted") return "danger"
  if (status === "running") return "info"
  return "warning"
}

export const jobActive = (job: Pick<Job, "status">) =>
  job.status === "queued" || job.status === "running"

/** The jobs the header's menu lists: those in progress first, then the latest, newest first. */
export function recentJobs<T extends Pick<Job, "status" | "created_at">>(jobs: T[], limit: number) {
  const newest = [...jobs].sort((a, b) => b.created_at - a.created_at)
  return [...newest.filter(jobActive), ...newest.filter((job) => !jobActive(job))].slice(0, limit)
}

/**
 * What a job worked on, by name. The manager records `container:<id>` or `image:<id>`, where a
 * catalog image's ID is its catalog ID; a deleted container keeps the name its deletion job
 * reported, or a short ID.
 */
export function jobTarget(
  job: Pick<Job, "resource" | "result">,
  containers: Map<string, string>,
  images = new Map<string, string>()
) {
  if (!job.resource) return ""
  // An update, a backup and a restore work on the whole installation.
  if (job.resource === "update" || job.resource === "installation") return "Thoric"
  const [kind, id] = job.resource.split(/:(.*)/s)
  const reported = job.result?.name as string | undefined
  // Jobs from before environments were called containers name them `environment:<id>`.
  if ((kind === "container" || kind === "environment") && id)
    return containers.get(id) ?? reported ?? t("jobs.target.container", { id: id.slice(0, 8) })
  // Downloads recorded before catalog images were listed with the others name `profile:<id>`.
  if (kind === "profile" && id) return images.get(id) ?? id
  if (kind === "image" && id)
    return images.get(id) ?? reported ?? t("jobs.target.image", { id: id.slice(0, 8) })
  return job.resource
}

/** The desktop a container runs: its image's desktop title, or the catalog image it names. */
export function desktopTitle(
  spec: ContainerSpec,
  title: string | undefined,
  images: Pick<ImageItem, "kind" | "reference" | "name">[]
) {
  return (
    title ||
    images.find((image) => image.kind === "catalog" && image.reference === spec.image)?.name ||
    t("containers.customDesktop")
  )
}

/** Where an image came from, as the image list names it. */
export function imageKind(kind: string) {
  const key = `images.kind.${kind}`
  return has(key) ? t(key) : kind
}

/** Whether an image can start a container now, and the color family for it. */
export function imageStatus(image: Pick<ImageItem, "kind" | "available" | "error">): {
  key: string
  tone: Tone
} {
  if (image.available === null || image.available === undefined)
    return { key: "unknown", tone: "muted" }
  if (image.available) return { key: "ready", tone: "success" }
  if (image.kind !== "catalog") return { key: "missing", tone: "danger" }
  // A catalog image that is not downloaded is fine: it downloads when it is used.
  return image.error ? { key: "unusable", tone: "danger" } : { key: "not_ready", tone: "muted" }
}

/**
 * What a container or a build names to start from an image: a catalog image by its reference, so
 * one that is not downloaded yet downloads first, and a saved image by its Docker ID.
 */
export const imageSource = (image: Pick<ImageItem, "kind" | "reference" | "image_id">) =>
  image.kind === "catalog" ? image.reference : (image.image_id ?? image.reference)

/** Whether an image's row offers Delete: a catalog image only once it is downloaded. */
export const deletableImage = (image: Pick<ImageItem, "kind" | "downloaded">) =>
  image.kind !== "catalog" || !!image.downloaded

/**
 * The names of the containers created from an image: the manager refuses to delete an image a
 * container starts from, since a reset creates the container from it again. An import keeps its
 * Docker image, and so does another saved image of the same Docker image, so nothing holds those.
 */
export function imageHolders(
  image: Pick<ImageItem, "id" | "kind" | "image_id">,
  containers: Pick<Container, "spec" | "source_image_id">[],
  images: Pick<ImageItem, "id" | "kind" | "image_id">[]
) {
  if (image.kind === "import" || !image.image_id) return []
  const kept = images.some(
    (other) =>
      image.kind !== "catalog" &&
      other.kind !== "catalog" &&
      other.id !== image.id &&
      other.image_id === image.image_id
  )
  if (kept) return []
  return containers
    .filter((container) => container.source_image_id === image.image_id)
    .map((container) => container.spec.name)
}

/**
 * What a deleted image's notice says. Docker keeps an image that another tag or image still holds,
 * which `removed` reports; an import keeps it by design, and an image that was gone already has
 * nothing left to keep.
 */
export const deletedNotice = (image: Pick<ImageItem, "kind" | "available">, removed: boolean) =>
  removed || image.kind === "import" || image.available === false
    ? "images.deleted"
    : "images.deletedKept"

export function jobLogText(job: Pick<Job, "log" | "status" | "result">) {
  if (job.log?.trim()) return job.log
  if (job.status === "queued") return t("jobs.log.queued")
  if (job.status === "running") return t("jobs.log.running")
  if (job.status === "completed") {
    if (job.result?.deleted) {
      const name = job.result.name as string | undefined
      const removed = (job.result.volumes_deleted as string[] | undefined) ?? []
      return [
        name ? t("jobs.log.deletedNamed", { name }) : t("jobs.log.deleted"),
        ...(removed.length ? [t("jobs.log.volumesDeleted", { names: removed.join(", ") })] : []),
        ...(job.result.volumes_preserved ? [t("jobs.log.volumesPreserved")] : []),
        ...(job.result.images_preserved ? [t("jobs.log.imagesPreserved")] : []),
      ].join(" ")
    }
    return t("jobs.log.completed")
  }
  return job.status === "failed" ? t("jobs.log.failed") : t("jobs.log.interrupted")
}

export type DownloadSummary = {
  text: string
  value?: number
  max?: number
  percent?: number
  /** What the bar measures, for its accessible name. */
  title?: string
}

// Docker's per-layer counts, summarized by the manager while a job downloads an image.
export function downloadProgress(progress?: DownloadProgress | null): DownloadSummary | null {
  if (!progress) return null
  const { current = 0, total = null, layers = 0, layers_done: done = 0 } = progress
  if (!layers) return { text: t("progress.preparing") }
  const counted = plural(
    layers,
    { one: "progress.layers.one", other: "progress.layers.other" },
    { done: formatNumber(done) }
  )
  if (!total) {
    const so =
      total === 0 ? t("progress.checking") : t("progress.downloaded", { size: fileSize(current) })
    return { text: `${so} · ${counted}` }
  }
  if (current >= total && done < layers)
    return { value: 1, max: 1, percent: 100, text: `${t("progress.extracting")} · ${counted}` }
  return {
    value: current,
    max: total,
    percent: Math.floor((current * 100) / total),
    text: `${t("progress.partOf", { current: fileSize(current), total: fileSize(total) })} · ${counted}`,
  }
}

/**
 * A running job's progress: layers for an image download, bytes for an archive or a volume copy,
 * and the step or line the scripts of a build or a compose setup reached.
 */
export function jobProgress(progress?: DownloadProgress | null): DownloadSummary | null {
  if (progress?.script) return scriptProgress(progress.script)
  if (!progress?.transfer) {
    const download = downloadProgress(progress)
    return download && { ...download, title: t("jobs.downloadProgress") }
  }
  const { current = 0, total = null } = progress
  const title = t("jobs.archiveProgress")
  if (progress.transfer === "export")
    return {
      title,
      text: current
        ? t("progress.exported", { size: fileSize(current) })
        : t("progress.preparingImage"),
    }
  if (progress.transfer === "copy")
    return { title, text: t("progress.copied", { size: fileSize(current) }) }
  if (!total) return { title, text: t("progress.received", { size: fileSize(current) }) }
  return {
    title,
    value: current,
    max: total,
    percent: Math.floor((current * 100) / total),
    text: t("progress.receivedOf", { current: fileSize(current), total: fileSize(total) }),
  }
}

/** The scripts' progress in words: the part when there are several, then the step or line. */
function scriptProgress(script: ScriptSummary): DownloadSummary {
  const { stage, stages, line, lines, label } = script
  const where =
    stage != null && stages != null
      ? t(script.skipped ? "progress.script.skipped" : "progress.script.stage", {
          stage: formatNumber(stage),
          stages: formatNumber(stages),
        })
      : line != null && lines != null
        ? t("progress.script.line", { line: formatNumber(line), lines: formatNumber(lines) })
        : undefined
  const part =
    script.parts > 1
      ? t("progress.script.part", {
          part: formatNumber(script.part),
          parts: formatNumber(script.parts),
          name: script.name ?? "",
        })
      : undefined
  const step = where && label ? `${where}: ${label}` : (where ?? label)
  return {
    title: t("jobs.scriptProgress"),
    value: script.percent,
    max: 100,
    percent: script.percent,
    text: [part, step].filter(Boolean).join(" · ") || t("progress.script.running"),
  }
}

/** A stage of a job in words; one this release does not know as the manager named it. */
export function stageLabel(name: string) {
  const key = `stage.${name}`
  return has(key) ? t(key) : name
}

export type StageState = "done" | "running" | "failed" | "pending" | "skipped"
export type StageRow = { name: string; label: string; state: StageState; seconds: number | null }

/**
 * A job's stages as the job panel lists them, with the time each took and the whole run's. Stages
 * only move forward, so one that did not start before the last started one was skipped, and the
 * last started stage of a failed or interrupted job is where it stopped. A running stage counts
 * up to `now`, in seconds like the manager's times.
 */
export function jobStages(job: Pick<Job, "status" | "stages" | "finished_at">, now: number) {
  const stages = job.stages ?? []
  const active = jobActive(job)
  const stopped = job.status === "failed" || job.status === "interrupted"
  const end = active ? now : (job.finished_at ?? now)
  let last = -1
  stages.forEach((stage, index) => {
    if (stage.started_at != null) last = index
  })
  const rows = stages.map(({ name, started_at, finished_at }, index): StageRow => {
    const label = stageLabel(name)
    if (started_at == null) {
      const skipped = index < last || job.status === "completed"
      return { name, label, state: skipped ? "skipped" : "pending", seconds: null }
    }
    const seconds = Math.max(0, (finished_at ?? end) - started_at)
    if (index === last && (stopped || (!active && finished_at == null)))
      return { name, label, state: "failed", seconds }
    return { name, label, state: finished_at == null ? "running" : "done", seconds }
  })
  const first = stages.find((stage) => stage.started_at != null)?.started_at
  return { rows, total: first == null ? null : Math.max(0, end - first) }
}

/** The stage a running job is in, in words, or nothing for a job without stages. */
export function currentStage(job: Pick<Job, "status" | "stages">) {
  if (job.status !== "running") return undefined
  const stage = job.stages?.findLast((stage) => stage.started_at != null)
  return stage && stageLabel(stage.name)
}

export type ScriptStepRow = {
  key: string
  /** Where the step is among the part's, such as `3/10`; none for a range of steps. */
  number?: string
  label: string
  state: StageState
  seconds: number | null
}
export type ScriptPartRow = {
  key: string
  name?: string
  state: StageState
  seconds: number | null
  cached: boolean
  steps: ScriptStepRow[]
}

/**
 * The parts of a job's scripts and the steps each named, as the job panel lists them. A number a
 * script jumped over, and the numbers after its last step in a part that ended well, were
 * skipped; in a part still running they are still to come, and in one a job stopped in they were
 * not reached. The step or part a failed or interrupted job stopped in has no end of its own.
 */
export function scriptParts(
  job: Pick<Job, "status" | "finished_at" | "script">,
  now: number
): ScriptPartRow[] {
  const active = jobActive(job)
  const stopped = job.status === "failed" || job.status === "interrupted"
  const end = active ? now : (job.finished_at ?? now)
  // A part or a step without an end is the one running, or the one a job stopped in.
  const open: StageState = active ? "running" : stopped ? "failed" : "done"
  const time = (from?: number | null, to?: number | null) =>
    from == null ? null : Math.max(0, (to ?? end) - from)
  const range = (from: number, to: number) =>
    from === to
      ? t("scriptSteps.step", { stage: formatNumber(from) })
      : t("scriptSteps.range", { from: formatNumber(from), to: formatNumber(to) })
  return (job.script?.parts ?? []).map((part, index): ScriptPartRow => {
    const ended = part.finished_at != null
    const steps: ScriptStepRow[] = []
    let last = 0
    for (const step of part.steps ?? []) {
      if (step.stage > last + 1)
        steps.push({
          key: `skipped-${last + 1}`,
          label: range(last + 1, step.stage - 1),
          state: "skipped",
          seconds: null,
        })
      steps.push({
        key: `step-${step.stage}`,
        number: part.stages
          ? `${formatNumber(step.stage)}/${formatNumber(part.stages)}`
          : formatNumber(step.stage),
        label: step.label ?? t("scriptSteps.step", { stage: formatNumber(step.stage) }),
        state: step.skipped
          ? "skipped"
          : step.started_at == null
            ? "pending"
            : step.finished_at != null || ended
              ? "done"
              : open,
        seconds: step.skipped ? null : time(step.started_at, step.finished_at ?? part.finished_at),
      })
      last = step.stage
    }
    if (part.stages && last && last < part.stages) {
      const count = part.stages - last
      steps.push({
        key: `rest-${last + 1}`,
        label: ended
          ? range(last + 1, part.stages)
          : plural(
              count,
              active
                ? { one: "scriptSteps.more.one", other: "scriptSteps.more.other" }
                : { one: "scriptSteps.notReached.one", other: "scriptSteps.notReached.other" }
            ),
        state: ended ? "skipped" : "pending",
        seconds: null,
      })
    }
    return {
      key: String(index),
      name: part.name,
      state:
        part.started_at == null
          ? job.status === "completed"
            ? "skipped"
            : "pending"
          : ended
            ? "done"
            : open,
      seconds: time(part.started_at, part.finished_at),
      cached: !!part.cached,
      steps,
    }
  })
}

/**
 * An image a form can choose; `catalog` marks one from the image catalog and `icon` is the URL of
 * a saved image's icon.
 */
export type ImageChoice = {
  value: string
  label: string
  disabled: boolean
  catalog: boolean
  icon?: string
}

type Choosable = Pick<
  ImageItem,
  "kind" | "reference" | "image_id" | "name" | "desktop" | "available" | "error"
> &
  Partial<Pick<ImageItem, "id" | "icon">>

export function imageChoices(
  images: Choosable[],
  defaultImage?: string,
  selected?: string
): ImageChoice[] {
  const choices = new Map<string, ImageChoice>()
  const catalog = images.filter((image) => image.kind === "catalog")
  // A catalog image downloads when it is used; one whose image does not match its labels cannot be used.
  for (const image of catalog)
    choices.set(image.reference, {
      value: image.reference,
      label:
        image.available || image.error
          ? image.name
          : `${image.name} · ${t("images.downloadsFirst")}`,
      disabled: !image.available && !!image.error,
      catalog: true,
    })
  if (defaultImage && !choices.has(defaultImage))
    choices.set(defaultImage, {
      value: defaultImage,
      label: defaultImage,
      disabled: false,
      catalog: false,
    })
  for (const image of images) {
    if (image.kind === "catalog" || !image.image_id) continue
    const icon = image.id ? iconUrl({ id: image.id, icon: image.icon }) : undefined
    choices.set(image.image_id, {
      value: image.image_id,
      label: image.desktop ? `${image.name} · ${image.desktop.title}` : image.name,
      disabled: image.available === false,
      catalog: false,
      ...(icon ? { icon } : {}),
    })
  }
  if (selected && !choices.has(selected))
    choices.set(selected, { value: selected, label: selected, disabled: false, catalog: false })
  return [...choices.values()]
}

/** The choice a form starts with: the requested one when usable, otherwise the first usable one. */
export function initialImage(choices: ImageChoice[], wanted?: string) {
  return choices.some((item) => item.value === wanted && !item.disabled)
    ? wanted
    : choices.find((item) => !item.disabled)?.value
}

type TreeRecord = Pick<ImageItem, "id" | "image_id" | "parent_image_id">

export function imageTree<T extends TreeRecord>(records: T[]): (T & { depth: number })[] {
  const ids = new Set(records.map((r) => r.image_id))
  const parents = new Map<string | null, T[]>()
  for (const record of records) {
    const parent = record.parent_image_id || null
    if (!parents.has(parent)) parents.set(parent, [])
    parents.get(parent)!.push(record)
  }
  const result: (T & { depth: number })[] = []
  const seen = new Set<string>()
  function visit(record: T, depth: number) {
    const stack: [T, number][] = [[record, depth]]
    while (stack.length) {
      const [item, level] = stack.pop()!
      if (seen.has(item.id)) continue
      seen.add(item.id)
      result.push({ ...item, depth: level })
      // A catalog image that is not downloaded has no Docker ID, so nothing builds on it.
      const children = (item.image_id && parents.get(item.image_id)) || []
      for (let i = children.length - 1; i >= 0; i--) stack.push([children[i], level + 1])
    }
  }
  for (const r of records) if (!r.parent_image_id || !ids.has(r.parent_image_id)) visit(r, 0)
  // Corrupt/cyclic metadata must not hide records or hang the UI.
  for (const r of records) if (!seen.has(r.id)) visit(r, 0)
  return result
}

/** What the settings dialog says before a change that recreates the container. */
export function restartNotice(fields: string[], running: boolean) {
  const names = fields.map((field) => {
    const key = `restart.field.${field}`
    return has(key) ? t(key) : field
  })
  const list = formatList(names)
  const subject = list.charAt(0).toLocaleUpperCase(getLocale()) + list.slice(1)
  return plural(
    names.length,
    { one: "restart.notice.one", other: "restart.notice.other" },
    {
      subject,
      effect: running ? t("restart.effect.running") : t("restart.effect.stopped"),
    }
  )
}

/** Finished jobs of one local day. */
export type ActivityPoint = { date: string; completed: number; failed: number }

/**
 * Finished jobs per local day over the last `days` days, oldest first, from the manager's hourly
 * counts, which outlive the job records. Interrupted jobs count as failed: either way the
 * operation did not finish as asked. Without `scripts` the script jobs are left out. An hour
 * belongs to the local day it begins in.
 */
export function jobActivity(
  hours: JobCount[],
  days: number,
  scripts = true,
  now = new Date()
): ActivityPoint[] {
  const key = (value: Date) =>
    `${value.getFullYear()}-${String(value.getMonth() + 1).padStart(2, "0")}-${String(value.getDate()).padStart(2, "0")}`
  const points = new Map<string, ActivityPoint>()
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (days - 1))
  for (let i = 0; i < days; i++) {
    const day = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i)
    points.set(key(day), { date: key(day), completed: 0, failed: 0 })
  }
  for (const hour of hours) {
    if (!scripts && hour.class === "script") continue
    const point = points.get(key(new Date(hour.at * 1000)))
    if (!point) continue
    point.completed += hour.completed
    point.failed += hour.failed + hour.interrupted
  }
  return [...points.values()]
}

/** Sum of CPU and memory limits over running containers; an unlimited container adds nothing. */
export function limitTotals(items: ContainerItem[]) {
  let cpus = 0
  let memoryMb = 0
  let unlimited = 0
  for (const item of items) {
    if (item.state !== "running") continue
    const { spec } = item
    if (spec.cpus) cpus += spec.cpus
    if (spec.memory_mb) memoryMb += spec.memory_mb
    if (!spec.cpus || !spec.memory_mb) unlimited++
  }
  return { cpus, memoryMb, unlimited }
}
