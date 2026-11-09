import { useSyncExternalStore } from "react"

import type { Job } from "@/lib/types"

/**
 * The kinds of the script class, as `SCRIPT_KINDS` in the manager's `jobs/history.rs` names them.
 * An agent runs scripts by the thousand, so the manager keeps them apart from the other jobs and a
 * list can leave them out.
 */
export const SCRIPT_KINDS = ["exec"]

export const isScript = (job: Pick<Job, "kind">) => SCRIPT_KINDS.includes(job.kind)

/** The query key of the job list without scripts, beside `jobs`, the list of every kind. */
export const OTHER_JOBS = "jobs-without-scripts"

/** The job list's path, of every kind or without the scripts. */
export const jobsPath = (scripts: boolean) => (scripts ? "jobs" : "jobs?scripts=false")

const KEY = "thoric.jobs-scripts"

/** The stored choice: scripts are shown unless it says otherwise. */
export const parseShowScripts = (value: string | null) => value !== "hidden"

let current: boolean | undefined
const listeners = new Set<() => void>()

function read() {
  if (current === undefined) {
    try {
      current = parseShowScripts(localStorage.getItem(KEY))
    } catch {
      current = true
    }
  }
  return current
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/**
 * Whether the job lists and the activity chart count scripts. The Jobs page, the header's menu
 * and the chart follow the same choice, kept in this browser.
 */
export const useShowScripts = () => useSyncExternalStore(subscribe, read, read)

export function setShowScripts(value: boolean) {
  current = value
  try {
    localStorage.setItem(KEY, value ? "shown" : "hidden")
  } catch {
    // Without storage the choice lasts until the page reloads.
  }
  for (const listener of listeners) listener()
}
