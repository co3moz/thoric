import { jobActive, jobName } from "./format"
import type { ContainerItem, ImageItem, Job } from "./types"

/** A container or an image a job is still making, under the name it will have. */
export type Pending = { id: string; name: string; job: Job }

/**
 * The jobs that make a container: a creation, a duplicate of another container and an instance of
 * a scale set.
 */
const containerJobs = new Set(["create", "duplicate", "scale-up"])

/**
 * The jobs that make a saved image, and the kind its record will have. A duplicate starts its
 * container from a snapshot it takes first.
 */
const imageJobs: Record<string, string> = {
  build: "build",
  snapshot: "snapshot",
  duplicate: "snapshot",
  "image-import": "archive",
}

/** The jobs that download or build a catalog image, whose row is listed all along. */
const catalogJobs = new Set(["image-pull", "source-build"])

/** The jobs whose end changes what the containers and images lists hold. */
const creationJobs = new Set([...containerJobs, ...Object.keys(imageJobs), ...catalogJobs])

/** The ID in a job's `container:<id>` or `image:<id>` resource. */
function resourceId(job: Pick<Job, "resource">, kind: "container" | "image") {
  const [prefix, id] = job.resource?.split(/:(.*)/s) ?? []
  return prefix === kind && id ? id : undefined
}

const pendingName = (job: Job, name?: string) => name || job.name || jobName(job.kind)

/**
 * The containers a creation job is still making: one with no record yet, or whose record has no
 * running container yet. A container whose desktop runs is listed as itself while its setup
 * finishes.
 */
export function pendingContainers(
  jobs: Job[],
  containers: Pick<ContainerItem, "id" | "state" | "spec">[]
): Pending[] {
  const records = new Map(containers.map((item) => [item.id, item]))
  return jobs.flatMap((job) => {
    const id =
      containerJobs.has(job.kind) && jobActive(job) ? resourceId(job, "container") : undefined
    if (!id) return []
    const record = records.get(id)
    if (record?.state === "running") return []
    return [{ id, name: pendingName(job, record?.spec.name), job }]
  })
}

/**
 * The saved images a job is still making, with the kind their records will have. A build, a
 * snapshot and a duplicate save their record under the job's ID, an import under the ID in its
 * resource; once the record is listed, it stands for itself.
 */
export function pendingImages(
  jobs: Job[],
  images: Pick<ImageItem, "id">[]
): (Pending & { kind: string })[] {
  const saved = new Set(images.map((image) => image.id))
  return jobs.flatMap((job) => {
    const kind = imageJobs[job.kind]
    if (!kind || !jobActive(job)) return []
    const id = job.kind === "image-import" ? resourceId(job, "image") : job.id
    if (!id || saved.has(id)) return []
    return [{ id, name: pendingName(job), job, kind }]
  })
}

/** The job downloading or building each catalog image, by catalog ID. */
export function catalogImageJobs(jobs: Job[]) {
  const found = new Map<string, Job>()
  for (const job of jobs) {
    const id = catalogJobs.has(job.kind) && jobActive(job) ? resourceId(job, "image") : undefined
    if (id && !found.has(id)) found.set(id, job)
  }
  return found
}

/**
 * What a pending record's badge says: queued, downloading an image, or creating. Only a download
 * counts its layers; a build's or a setup's scripts and an archive transfer report progress too.
 */
export function pendingState(job: Pick<Job, "kind" | "status" | "progress">) {
  if (job.status === "queued") return "queued"
  const downloading = job.kind === "image-pull" || job.progress?.layers != null
  return downloading ? "downloading" : "creating"
}

/** Whether a job that makes a container or an image ended between two reads of the job list. */
export function creationEnded(before: Job[], after: Job[]) {
  const now = new Map(after.map((job) => [job.id, job]))
  return before.some((job) => {
    if (!creationJobs.has(job.kind) || !jobActive(job)) return false
    const later = now.get(job.id)
    return !later || !jobActive(later)
  })
}
