// What the Duplicate dialog offers: a name no container has yet, and for each volume the container
// mounts, whether the duplicate gets a copy of its own or mounts the same one.
import type { Container, Mount, Volume } from "./types"
import { ownedVolumes } from "./volume-files"

/** The longest container name the manager takes. */
const NAME_LIMIT = 100

/** How the manager compares the names of containers: without case and the spaces around them. */
const nameKey = (name: string) => name.trim().toLowerCase()

/** A container, or one a job is still making, by its name. */
type Named = { spec: { name: string } }

/** Whether a container in `containers` is called `name` already, which the manager refuses. */
export const nameTaken = (name: string, containers: Named[]) =>
  containers.some((container) => nameKey(container.spec.name) === nameKey(name))

/**
 * A free name for a duplicate of the container called `name`: `<name>-copy`, or `<name>-copy-2`
 * and on while those are taken, within the manager's limit.
 */
export function duplicateName(name: string, containers: Named[]) {
  for (let count = 1; ; count++) {
    const suffix = count === 1 ? "-copy" : `-copy-${count}`
    const candidate = name.trim().slice(0, NAME_LIMIT - suffix.length) + suffix
    if (!nameTaken(candidate, containers)) return candidate
  }
}

/**
 * The volumes the container mounts, each once with the folder it is mounted at: `copied` are those
 * created for it, which the duplicate gets copies of, and `shared` every other one, which both
 * containers mount.
 */
export function duplicateVolumes(
  container: Pick<Container, "id" | "spec" | "setup">,
  volumes: Pick<Volume, "Name" | "owner">[]
) {
  const owned = new Set(ownedVolumes(volumes, container.id))
  const seen = new Set<string>()
  const copied: Mount[] = []
  const shared: Mount[] = []
  for (const mount of [...container.spec.mounts, ...(container.setup?.mounts ?? [])]) {
    if (seen.has(mount.volume)) continue
    seen.add(mount.volume)
    if (owned.has(mount.volume)) copied.push(mount)
    else shared.push(mount)
  }
  return { copied, shared }
}
