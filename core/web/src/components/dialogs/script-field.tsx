import * as React from "react"
import { RiFileCodeLine, RiFullscreenLine, RiTerminalBoxLine } from "@remixicon/react"
import { cn } from "cn"

import { CodeEditor } from "@/components/code-editor"
import { ColoredCode } from "@/components/code-textarea"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import type { CodeLanguage } from "@/lib/highlight"
import { plural, t } from "@/lib/i18n"
import { tile } from "./form-sections"

/** The lines a script's card shows of it. */
const PREVIEW_LINES = 5

/** One script the editor opens, by a key its caller knows it by. */
export type Script = { key: string; title: string; value: string }

/**
 * A script in a form as a short preview that does not scroll, so the form has one scrollbar, and
 * opens the script in an editor over the dialog. `id` goes to a stand-in the form checks, so an
 * empty required script stops the form and, when the browser points at it, opens the editor.
 */
export function ScriptField({
  id,
  label,
  language = "bash",
  value,
  onChange,
  required,
  hint,
}: {
  id?: string
  label: string
  language?: CodeLanguage
  value: string
  onChange: (value: string) => void
  required?: boolean
  /** How the script runs, under the card and over the editor. */
  hint?: string
}) {
  const [open, setOpen] = React.useState(false)
  return (
    <>
      <ScriptCard
        id={id}
        label={label}
        language={language}
        value={value}
        required={required}
        onOpen={() => setOpen(true)}
      />
      <ScriptEditor
        open={open}
        onOpenChange={setOpen}
        scripts={[{ key: "script", title: label, value }]}
        start="script"
        language={language}
        hint={hint}
        onDone={(values) => onChange(values.script)}
      />
    </>
  )
}

/** A script's first lines, how long it is, and the way into its editor. */
export function ScriptCard({
  id,
  label,
  language,
  value,
  required,
  onOpen,
}: {
  id?: string
  label: string
  language: CodeLanguage
  value: string
  required?: boolean
  onOpen: () => void
}) {
  const lines = value ? value.split("\n").length : 0
  const missing = required && !value.trim()
  const button = React.useRef<HTMLButtonElement>(null)
  return (
    <div className="relative">
      <button
        ref={button}
        type="button"
        onClick={onOpen}
        aria-label={t("script.editOf", { name: label })}
        className={cn(
          "group flex w-full flex-col overflow-hidden text-left transition-colors outline-none hover:bg-input/70 focus-visible:ring-[3px] focus-visible:ring-ring/50 dark:hover:bg-muted",
          tile,
          missing && "ring-1 ring-destructive"
        )}
      >
        <pre
          aria-hidden
          className="m-0 h-[6.25rem] overflow-hidden [mask-image:linear-gradient(to_bottom,black_55%,transparent)] px-3 pt-3 font-mono text-xs leading-5 whitespace-pre text-foreground"
        >
          {value ? (
            <ColoredCode
              value={value.split("\n").slice(0, PREVIEW_LINES).join("\n")}
              language={language}
            />
          ) : (
            <span className="text-muted-foreground">{t("script.empty")}</span>
          )}
        </pre>
        <span className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground">
          <RiFileCodeLine aria-hidden className="size-3.5 shrink-0" />
          {missing ? (
            <span className="text-destructive">{t("setup.required")}</span>
          ) : (
            plural(lines, { one: "script.lines.one", other: "script.lines.other" })
          )}
          <span className="ml-auto inline-flex items-center gap-1.5 rounded-full bg-background/70 px-2.5 py-1 font-medium text-foreground group-hover:bg-background">
            <RiFullscreenLine aria-hidden className="size-3.5" />
            {t("script.edit")}
          </span>
        </span>
      </button>
      {/* A stand-in for the form, which cannot see the script: an empty required one stops it,
          and the browser's pointing at it opens the editor. */}
      <textarea
        id={id}
        required={required}
        value={value}
        onChange={() => {}}
        onFocus={() => {
          // The editor gives focus back to what had it, which must be the card, not this.
          button.current?.focus()
          onOpen()
        }}
        // A window without focus hears no focus event, so the refusal itself opens the editor,
        // when this is the field the browser points at.
        onInvalid={(event) => {
          const field = event.currentTarget
          const first = [...(field.form?.elements ?? [])].find(
            (element) => "validity" in element && !(element as HTMLInputElement).validity.valid
          )
          if (first !== field) return
          button.current?.focus()
          onOpen()
        }}
        tabIndex={-1}
        aria-hidden
        className="pointer-events-none absolute bottom-0 left-4 size-px resize-none opacity-0"
      />
    </div>
  )
}

/**
 * The editor of one or several scripts, such as the parts of a merged build, over the dialog
 * that holds them and nearly as large as the window. It edits copies: Done hands back every
 * script by its key, and closing otherwise drops the changes, asking first when there are any.
 */
export function ScriptEditor({
  open,
  onOpenChange,
  scripts,
  start,
  language,
  hint,
  onDone,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  scripts: Script[]
  /** The key of the script shown first. */
  start: string
  language: CodeLanguage
  hint?: string
  onDone: (values: Record<string, string>) => void
}) {
  const [drafts, setDrafts] = React.useState<Record<string, string>>({})
  const [current, setCurrent] = React.useState(start)
  const [asking, setAsking] = React.useState(false)
  const [wasOpen, setWasOpen] = React.useState(open)
  // Each opening starts from the scripts as the form holds them.
  if (open !== wasOpen) {
    setWasOpen(open)
    if (open) {
      setDrafts(Object.fromEntries(scripts.map((script) => [script.key, script.value])))
      setCurrent(start)
    }
  }
  const changed = scripts.some((script) => (drafts[script.key] ?? script.value) !== script.value)
  const shown = scripts.find((script) => script.key === current) ?? scripts[0]
  const text = drafts[shown.key] ?? shown.value
  const lines = text ? text.split("\n").length : 0

  const done = () => {
    onDone(
      Object.fromEntries(scripts.map((script) => [script.key, drafts[script.key] ?? script.value]))
    )
    onOpenChange(false)
  }
  const close = (next: boolean) => {
    if (next) return onOpenChange(true)
    if (changed) setAsking(true)
    else onOpenChange(false)
  }

  return (
    <>
      <Dialog open={open} onOpenChange={close}>
        <DialogContent className="top-4 right-4 bottom-4 left-4 flex w-auto max-w-none translate-x-0 translate-y-0 flex-col gap-0 p-0 sm:max-w-none">
          <DialogHeader className="flex-row flex-wrap items-center gap-x-4 gap-y-2 px-6 pt-5 pr-14 pb-4">
            <div className="flex min-w-0 items-center gap-3">
              <RiTerminalBoxLine aria-hidden className="size-5 shrink-0 text-muted-foreground" />
              <div className="flex min-w-0 flex-col gap-1">
                <DialogTitle className="truncate">{shown.title}</DialogTitle>
                {hint && <DialogDescription className="text-xs">{hint}</DialogDescription>}
              </div>
            </div>
            {scripts.length > 1 && (
              <div role="group" aria-label={t("script.scripts")} className="flex flex-wrap gap-1">
                {scripts.map((script, index) => (
                  <Button
                    key={script.key}
                    type="button"
                    variant="ghost"
                    size="sm"
                    aria-pressed={script.key === shown.key}
                    onClick={() => setCurrent(script.key)}
                    className="text-muted-foreground aria-pressed:bg-muted aria-pressed:text-foreground"
                  >
                    {`${index + 1} · ${script.title}`}
                  </Button>
                ))}
              </div>
            )}
          </DialogHeader>
          <div className="flex min-h-0 flex-1 px-6">
            <CodeEditor
              // Each script is an editor of its own, with its own history.
              key={shown.key}
              fill
              language={language}
              label={shown.title}
              value={text}
              onChange={(value) => setDrafts((all) => ({ ...all, [shown.key]: value }))}
              onSave={done}
              autoFocus
              className="min-h-0 flex-1"
            />
          </div>
          <DialogFooter className="items-center px-6 pt-4 pb-5 sm:items-center">
            <p className="mr-auto hidden text-xs text-muted-foreground sm:block">
              {plural(lines, { one: "script.lines.one", other: "script.lines.other" })}
              {" · "}
              {t("script.saveKeys")}
            </p>
            <Button type="button" variant="outline" onClick={() => close(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="button" onClick={done}>
              {t("script.done")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <AlertDialog open={asking} onOpenChange={setAsking}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("unsaved.changes.title")}</AlertDialogTitle>
            <AlertDialogDescription>{t("script.discardHint")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("unsaved.changes.keep")}</AlertDialogCancel>
            <Button
              variant="destructive"
              onClick={() => {
                setAsking(false)
                onOpenChange(false)
              }}
            >
              {t("unsaved.changes.discard")}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
