import * as React from "react"
import { cn } from "cn"

import { CodeTextarea, CodeView } from "@/components/code-textarea"
import { Textarea } from "@/components/ui/textarea"
import {
  editorHeight,
  holdsFixed,
  LINE_HEIGHT,
  modelName,
  modelUri,
  monacoLanguage,
  PADDING,
} from "@/lib/code-editor"
import type { CodeLanguage } from "@/lib/highlight"
import { t } from "@/lib/i18n"
import type { JsonSchema } from "@/lib/schema-form"
import type { ScriptContext } from "@/lib/script-completions"

type Setup = typeof import("./monaco-setup")
type Editor = ReturnType<Setup["monaco"]["editor"]["create"]>

let loading: Promise<Setup> | undefined
let loaded: Setup | undefined

/**
 * Monaco, once its chunk has loaded. Until then, after it failed, and on a touch screen, which
 * Monaco does not support, the fields fall back to `CodeTextarea` and `CodeView`.
 */
function useMonaco(): Setup | undefined {
  // Asked when each field appears, so a laptop that turned into a tablet gets the plain field.
  const [touch] = React.useState(() => window.matchMedia("(pointer: coarse)").matches)
  const [setup, setSetup] = React.useState(() => (touch ? undefined : loaded))
  React.useEffect(() => {
    if (setup || touch) return
    let live = true
    loading ??= import("./monaco-setup").then((module) => (loaded = module))
    loading.then(
      (module) => live && setSetup(module),
      () => (loading = undefined)
    )
    return () => {
      live = false
    }
  }, [setup, touch])
  return setup
}

type Common = {
  value: string
  /** What the field is, for screen readers. */
  label: string
  /** A language the interface colors itself; without one, the one `path` names, if any. */
  language?: CodeLanguage
  /** The file the text is, whose name tells Monaco its language, such as `.py` or `Dockerfile`. */
  path?: string
  /** The schema JSON is checked against, with completion and hovers from it. */
  schema?: JsonSchema
  /** Where a shell script runs, which completes the commands and values it has there. */
  completions?: ScriptContext
  /** The least and the most the field grows to, in pixels; it scrolls past the most. */
  minHeight?: number
  maxHeight?: number
  /** Long lines wrap rather than scroll sideways; JSON's do unless told otherwise. */
  wrap?: boolean
  className?: string
}

type Editable = {
  onChange(value: string): void
  /** Ctrl+Enter, which Monaco would otherwise take for a new line below. */
  onSubmit?(): void
  /** Ctrl+S, which the browser would otherwise take for saving the page. */
  onSave?(): void
  placeholder?: string
  invalid?: boolean
  /** The field's id, so its label focuses it. */
  id?: string
  /** An empty editor stops its form, as an empty required textarea does. */
  required?: boolean
  autoFocus?: boolean
  /** The editor takes the height its class gives it instead of growing with its text. */
  fill?: boolean
}

/**
 * Code to edit in Monaco, the editor of VS Code, once it has loaded: a script, a configuration
 * or a file, colored as its language, and JSON checked against its schema.
 */
export function CodeEditor(props: Common & Editable) {
  const setup = useMonaco()
  if (setup) return <Monaco setup={setup} {...props} />
  const { language, value, label, onChange, invalid, minHeight, maxHeight, fill, className } = props
  const field = {
    id: props.id,
    required: props.required,
    autoFocus: props.autoFocus,
    placeholder: props.placeholder,
    value,
    "aria-label": label,
    "aria-invalid": invalid || undefined,
    onChange: (event: React.ChangeEvent<HTMLTextAreaElement>) => onChange(event.target.value),
    style: fill ? undefined : { minHeight, maxHeight },
  }
  if (language)
    return <CodeTextarea {...field} language={language} className={fill ? className : undefined} />
  return (
    <Textarea
      {...field}
      spellCheck={false}
      className={cn("resize-none overflow-y-auto font-mono text-xs", fill && className)}
    />
  )
}

/** Code to read, with its blocks folding; Monaco once it has loaded. */
export function CodeReader({ language, ...common }: Common & { language: CodeLanguage }) {
  const setup = useMonaco()
  if (!setup)
    return (
      <CodeView
        language={language}
        value={common.value}
        label={common.label}
        className={common.className}
      />
    )
  return <Monaco setup={setup} language={language} {...common} readOnly />
}

/** The elements above `node`, nearest first. */
function ancestors(node: HTMLElement) {
  const found: HTMLElement[] = []
  for (let parent = node.parentElement; parent; parent = parent.parentElement) found.push(parent)
  return found
}

function Monaco({
  setup,
  value,
  label,
  language,
  path,
  schema,
  completions,
  minHeight = 36,
  maxHeight = 384,
  readOnly = false,
  onChange,
  onSubmit,
  onSave,
  placeholder,
  invalid,
  id,
  required,
  autoFocus,
  fill,
  wrap,
  className,
}: Common &
  Partial<Editable> & {
    setup: Setup
    readOnly?: boolean
  }) {
  const frame = React.useRef<HTMLDivElement>(null)
  const host = React.useRef<HTMLDivElement>(null)
  const editor = React.useRef<Editor | null>(null)
  const key = React.useId().replace(/[^A-Za-z0-9]/g, "")
  const uri = modelUri(key, modelName(path, language))
  const languageId = language && monacoLanguage(language)
  // What the editor reads when it is made and when it calls back, kept current without
  // making it again; the effects below pass later changes on.
  const props = {
    value,
    onChange,
    onSubmit,
    onSave,
    minHeight,
    maxHeight,
    readOnly,
    label,
    placeholder,
    autoFocus,
    fill,
    wrap,
  }
  const latest = React.useRef(props)
  React.useEffect(() => {
    latest.current = props
  })

  React.useEffect(() => {
    const { monaco } = setup
    const first = latest.current
    const json = languageId === "json"
    // Suggestions and hovers float over the page by the window's coordinates, unless something
    // above, such as a dialog, would misplace them: there they stay in the editor's own box,
    // which then lets them out.
    const fixed = !ancestors(host.current!).some((node) => holdsFixed(getComputedStyle(node)))
    host.current!.style.overflow = fixed ? "hidden" : "visible"
    // Without a language, Monaco tells it from the model's name, or from a `#!` line.
    const model = monaco.editor.createModel(first.value, languageId, monaco.Uri.parse(uri))
    const created = monaco.editor.create(host.current!, {
      model,
      readOnly: first.readOnly,
      domReadOnly: first.readOnly,
      ariaLabel: first.label,
      placeholder: first.placeholder,
      automaticLayout: true,
      fixedOverflowWidgets: fixed,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: (first.wrap ?? json) ? "on" : "off",
      lineNumbers: first.readOnly ? "off" : "on",
      lineNumbersMinChars: 2,
      lineDecorationsWidth: 6,
      glyphMargin: false,
      folding: true,
      renderLineHighlight: "none",
      overviewRulerLanes: 0,
      overviewRulerBorder: false,
      hideCursorInOverviewRuler: true,
      stickyScroll: { enabled: false },
      fontFamily: getComputedStyle(document.documentElement).getPropertyValue("--font-mono"),
      fontSize: 12,
      lineHeight: LINE_HEIGHT,
      padding: { top: PADDING, bottom: PADDING },
      tabSize: 2,
      // JSON completes from its schema; other text from its own words, when asked with
      // Ctrl+Space, so a script does not pop a list up at every word typed.
      wordBasedSuggestions: json ? "off" : "currentDocument",
      quickSuggestions: json ? { strings: true, other: true, comments: false } : false,
      // A field that does not scroll lets the page scroll under the wheel.
      scrollbar: {
        alwaysConsumeMouseWheel: false,
        verticalScrollbarSize: 8,
        horizontalScrollbarSize: 8,
      },
    })
    editor.current = created
    // The field grows with its text and shows a scrollbar only once the text is taller; one that
    // fills its frame scrolls as any editor does.
    let scrolls: boolean | undefined
    const fit = () => {
      const { minHeight: least, maxHeight: most, fill: fills } = latest.current
      if (fills) return
      const content = created.getContentHeight()
      // The frame's border is outside the text's height.
      if (frame.current) frame.current.style.height = `${editorHeight(content, least, most) + 2}px`
      if (scrolls !== content > most) {
        scrolls = content > most
        created.updateOptions({ scrollbar: { vertical: scrolls ? "auto" : "hidden" } })
      }
    }
    const resize = created.onDidContentSizeChange(fit)
    fit()
    const changes = model.onDidChangeContent(() => latest.current.onChange?.(model.getValue()))
    if (first.onSubmit)
      created.addAction({
        id: "thoric.submit",
        label: t("inspector.run.keys"),
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
        run: () => latest.current.onSubmit?.(),
      })
    if (first.onSave)
      created.addAction({
        id: "thoric.save",
        label: t("common.save"),
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS],
        run: () => latest.current.onSave?.(),
      })
    if (first.autoFocus) created.focus()
    // A dialog hears Escape on its way in, before the editor, and would close over an open
    // suggestion list or search. The editor takes Escape from it, and what the editor leaves
    // unused, which Monaco does not stop, it hands back from its frame, so Escape still closes a
    // dialog from a field with nothing open.
    const node = host.current!
    const mine = (event: KeyboardEvent) =>
      event.key === "Escape" && node.contains(event.target as Node)
    const take = (event: KeyboardEvent) => mine(event) && event.preventDefault()
    const handBack = (event: KeyboardEvent) =>
      mine(event) &&
      frame.current?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })
      )
    window.addEventListener("keydown", take, true)
    window.addEventListener("keydown", handBack)
    return () => {
      window.removeEventListener("keydown", take, true)
      window.removeEventListener("keydown", handBack)
      changes.dispose()
      resize.dispose()
      created.dispose()
      model.dispose()
      editor.current = null
    }
  }, [setup, uri, languageId])

  // A value set from outside, such as Clear or the form, replaces the text.
  React.useEffect(() => {
    const model = editor.current?.getModel()
    if (model && model.getValue() !== value) model.setValue(value)
  }, [value])

  React.useEffect(() => {
    editor.current?.updateOptions({
      readOnly,
      domReadOnly: readOnly,
      ariaLabel: label,
      placeholder,
    })
  }, [readOnly, label, placeholder])

  // Schemas are compared as text: a caller may make an equal one on every render.
  const schemaText = schema ? JSON.stringify(schema) : ""
  React.useEffect(() => {
    setup.setSchema(uri, schemaText ? JSON.parse(schemaText) : undefined)
    return () => setup.setSchema(uri, undefined)
  }, [setup, uri, schemaText])

  // Compared as text too, since a caller makes its variables anew on every render.
  const completionsText = completions ? JSON.stringify(completions) : ""
  React.useEffect(() => {
    setup.setScriptContext(uri, completionsText ? JSON.parse(completionsText) : undefined)
    return () => setup.setScriptContext(uri, undefined)
  }, [setup, uri, completionsText])

  return (
    <div
      ref={frame}
      data-slot="code-editor"
      style={fill ? undefined : { height: minHeight + 2 }}
      className={cn(
        "relative w-full min-w-0 rounded-xl border border-input bg-input/30 transition-colors focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/50",
        invalid &&
          "border-destructive ring-[3px] ring-destructive/20 dark:border-destructive/50 dark:ring-destructive/40",
        className
      )}
    >
      <div ref={host} className="size-full rounded-[inherit]" />
      {!readOnly && (
        // A stand-in for the form, which cannot see into Monaco: it carries the field's id, so
        // its label focuses the editor, and `required`, so an empty editor stops the form the
        // way an empty textarea would, with the browser's note at the editor's foot.
        <textarea
          id={id}
          required={required}
          value={value}
          onChange={() => {}}
          onFocus={() => editor.current?.focus()}
          tabIndex={-1}
          aria-hidden
          className="pointer-events-none absolute bottom-0 left-4 size-px resize-none opacity-0"
        />
      )}
    </div>
  )
}
