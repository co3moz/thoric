import { useSyncExternalStore } from "react"

/** How often a container's preview asks for a new picture, in seconds; 0 keeps the first one. */
export const PREVIEW_REFRESH = [1, 2, 5, 10, 30, 0] as const
export type PreviewRefresh = (typeof PREVIEW_REFRESH)[number]

const DEFAULT: PreviewRefresh = 10
const KEY = "thoric.preview-refresh"

/** The stored choice, or the default for anything else. */
export function parseRefresh(value: string | null): PreviewRefresh {
  if (!value || !/^\d+$/.test(value)) return DEFAULT
  const seconds = Number(value)
  return PREVIEW_REFRESH.find((choice) => choice === seconds) ?? DEFAULT
}

let current: PreviewRefresh | undefined
const listeners = new Set<() => void>()

function read(): PreviewRefresh {
  if (current === undefined) {
    try {
      current = parseRefresh(localStorage.getItem(KEY))
    } catch {
      current = DEFAULT
    }
  }
  return current
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** Every preview on the page follows the same choice, kept in this browser. */
export const usePreviewRefresh = () => useSyncExternalStore(subscribe, read, read)

export function setPreviewRefresh(value: PreviewRefresh) {
  current = value
  try {
    localStorage.setItem(KEY, String(value))
  } catch {
    // Without storage the choice lasts until the page reloads.
  }
  for (const listener of listeners) listener()
}

/**
 * A queue of `size` turns. `take` calls `onTurn` with the function that gives the turn back once
 * one is free, and returns the function that leaves the queue, or gives the turn back once it was
 * given; either can be called more than once.
 */
export function turns(size: number) {
  let free = size
  const waiting: Array<() => void> = []
  return function take(onTurn: (done: () => void) => void): () => void {
    let state: "waiting" | "held" | "done" = "waiting"
    const done = () => {
      if (state !== "held") return
      state = "done"
      const next = waiting.shift()
      if (next) next()
      else free++
    }
    const grant = () => {
      state = "held"
      onTurn(done)
    }
    if (free > 0) {
      free--
      grant()
    } else waiting.push(grant)
    return () => {
      if (state === "waiting") {
        state = "done"
        waiting.splice(waiting.indexOf(grant), 1)
      } else done()
    }
  }
}

/**
 * At most two previews load a picture at once. The manager takes two screenshots at a time and
 * refuses one that waited too long, so a fast refresh of many previews waits its turn here instead,
 * and an agent's screenshot still gets one of them soon.
 */
export const previewTurn = turns(2)
