import { createContext, useContext, useEffect, useState, useSyncExternalStore } from "react"

import type { BackupManifest, BackupReview } from "./backup"
import type { ImportedBuild } from "./build-config"
import type { EndpointToken } from "./types"

export type Dialog =
  | { kind: "create"; image?: string }
  | { kind: "details"; id: string }
  | { kind: "settings"; id: string }
  | { kind: "snapshot"; id: string }
  | { kind: "duplicate"; id: string }
  | { kind: "delete"; id: string }
  | { kind: "reset"; id: string }
  /** Containers to connect to a network group: `group`, or a new one named in the dialog. */
  | { kind: "group-connect"; group?: string }
  | { kind: "upload"; id: string; files?: File[] }
  | { kind: "download"; id: string }
  | { kind: "build"; config?: ImportedBuild; simple?: boolean }
  | { kind: "compose"; id: string }
  | { kind: "icon"; id: string }
  | { kind: "image"; id: string }
  | { kind: "image-delete"; id: string }
  | { kind: "import" }
  | { kind: "volume" }
  | { kind: "volume-files"; name: string }
  | { kind: "volume-import" }
  | { kind: "volume-copy"; name: string }
  | { kind: "volume-delete"; name: string }
  | { kind: "job"; id: string }
  | { kind: "log"; title: string; text: string; status?: string }
  | { kind: "mcp-endpoint"; name?: string }
  | { kind: "mcp-token"; token: EndpointToken }
  | { kind: "mcp-endpoint-confirm"; name: string; action: "replace" | "revoke" | "delete" }
  | { kind: "mcp-tools"; name: string }
  /** Client settings for an endpoint, or for Thoric MCP at `/mcp` without a name. */
  | { kind: "mcp-connect"; name?: string }
  | { kind: "mcp-server"; container?: string }
  | { kind: "mcp-server-remove"; container: string; name: string }
  /** The agent activity of an MCP endpoint. */
  | { kind: "activity"; endpoint: string }
  | { kind: "update" }
  /** The confirmation of a restore, with what the backup holds and what is in its way. */
  | {
      kind: "restore"
      file: File
      manifest: BackupManifest
      review: BackupReview
      limit: number
    }

/** What closing a dialog or leaving a page would lose: a form's changes, or uploads still going. */
export type Unsaved = "changes" | "upload"

/** A question waiting for an answer: `then` runs once the user agrees to lose it, `keep` if not. */
export type Discard = { reason: Unsaved; then: () => void; keep?: () => void }

type State = { dialog: Dialog | null; open: boolean; seq: number; asking: Discard | null }

// One dialog at a time, as before: opening another replaces it. The last one stays mounted while
// it closes so its exit animation can run; `seq` remounts it with fresh form state when reopened.
let state: State = { dialog: null, open: false, seq: 0, asking: null }
const listeners = new Set<() => void>()
const emit = () => listeners.forEach((listener) => listener())

/** What dialogs and pages hold now, by the `seq` of their dialog, or `null` for the page itself. */
const holds = new Set<{ seq: number | null; reason: Unsaved }>()

/**
 * Opens `dialog` in place of any other and returns the number of this opening, which its holds
 * name. Code that opens one on its own, such as a job's log after a form was sent, replaces the
 * form as it is; a button outside it goes through `whenDiscarded`.
 */
export function openDialog(dialog: Dialog) {
  state = { dialog, open: true, seq: state.seq + 1, asking: null }
  emit()
  return state.seq
}

/** The dialog open now, if any. */
export const openedDialog = () => (state.open ? state.dialog : null)

/** Closes the open dialog as it is, as a sent form or the end of the session does. */
export function closeDialog() {
  if (!state.open && !state.asking) return
  state = { ...state, open: false, asking: null }
  emit()
}

function held(seq: number | null): Unsaved | null {
  let found: Unsaved | null = null
  for (const hold of holds)
    if (hold.seq === seq) {
      if (hold.reason === "upload") return "upload"
      found = hold.reason
    }
  return found
}

/** What the open dialog would lose by closing. */
export const dialogUnsaved = () => (state.open ? held(state.seq) : null)

/** Marks what the dialog `seq`, or the page for `null`, would lose; returns what unmarks it. */
export function hold(seq: number | null, reason: Unsaved) {
  const entry = { seq, reason }
  holds.add(entry)
  return () => void holds.delete(entry)
}

function ask(reason: Unsaved, then: () => void, keep?: () => void) {
  state = { ...state, asking: { reason, then, keep } }
  emit()
}

/** The question waiting for an answer, if any. */
export const discardQuestion = () => state.asking

/**
 * Closes the open dialog because its user asked to, with Escape, a click outside, its close
 * button or Cancel: at once, or once they agree to lose what it holds.
 */
export function dismissDialog() {
  const reason = dialogUnsaved()
  if (reason) ask(reason, closeDialog)
  else closeDialog()
}

/**
 * Runs `then`, which takes the open dialog's place, such as a toast's button: at once, or once the
 * user agrees to lose what the dialog holds.
 */
export function whenDiscarded(then: () => void) {
  const reason = dialogUnsaved()
  if (reason) ask(reason, then)
  else then()
}

/**
 * Leaves the page for another, or with `within` moves within it, such as to another container's
 * files: `then` runs at once, or once the user agrees to lose what the page holds, and its dialog
 * too when the page goes. `keep` runs when they stay.
 */
export function whenLeaving(then: () => void, keep: () => void, within = false) {
  const reason = (within ? null : dialogUnsaved()) ?? held(null)
  if (reason) ask(reason, then, keep)
  else then()
}

/** The user agreed to lose it: the dialog closes as it is and what waited runs. */
export function confirmDiscard() {
  const asking = state.asking
  if (!asking) return
  state = { ...state, open: false, asking: null }
  emit()
  asking.then()
}

/** The user keeps what the dialog or the page holds. */
export function keepEditing() {
  const asking = state.asking
  if (!asking) return
  state = { ...state, asking: null }
  emit()
  asking.keep?.()
}

/** The `seq` of the dialog a component renders in, which `DialogHost` provides. */
export const DialogScope = createContext<number | null>(null)

/**
 * Marks what closing the dialog this renders in, or leaving its page outside a dialog, would lose
 * while `reason` is set, so that is asked first.
 */
export function useUnsaved(reason: Unsaved | null | false) {
  const seq = useContext(DialogScope)
  useEffect(() => (reason ? hold(seq, reason) : undefined), [seq, reason])
}

/** Marks a form's changes: `values` differ from what they were when it opened. */
export function useDraft(values: unknown) {
  const text = JSON.stringify(values)
  const [first] = useState(text)
  useUnsaved(text !== first && "changes")
}

export function useDialogState() {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
    () => state
  )
}
