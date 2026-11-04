// Monaco, loaded the first time a code editor shows, so no page carries it before: the editor
// core, the features its editors use, the JSON language service, the coloring of every language
// Monaco knows, and both workers built from the page's own origin. A language's coloring is a
// small chunk of its own, loaded when text in it first opens; the language services of
// TypeScript, CSS and HTML, with their large workers, stay out.
import * as monaco from "monaco-editor/editor/editor.api.js"
import "monaco-editor/editor/browser/coreCommands.js"
import "monaco-editor/editor/browser/widget/codeEditor/codeEditorWidget.js"
import "monaco-editor/editor/contrib/bracketMatching/browser/bracketMatching.js"
import "monaco-editor/editor/contrib/clipboard/browser/clipboard.js"
import "monaco-editor/editor/contrib/contextmenu/browser/contextmenu.js"
import "monaco-editor/editor/contrib/cursorUndo/browser/cursorUndo.js"
import "monaco-editor/features/find/register.js"
import "monaco-editor/editor/contrib/folding/browser/folding.js"
import "monaco-editor/editor/contrib/format/browser/formatActions.js"
import "monaco-editor/editor/contrib/gotoError/browser/gotoError.js"
import "monaco-editor/editor/contrib/hover/browser/hoverContribution.js"
import "monaco-editor/editor/contrib/indentation/browser/indentation.js"
import "monaco-editor/editor/contrib/linesOperations/browser/linesOperations.js"
import "monaco-editor/editor/contrib/links/browser/links.js"
import "monaco-editor/editor/contrib/multicursor/browser/multicursor.js"
import "monaco-editor/editor/contrib/placeholderText/browser/placeholderText.contribution.js"
import "monaco-editor/editor/contrib/readOnlyMessage/browser/contribution.js"
import "monaco-editor/editor/contrib/smartSelect/browser/smartSelect.js"
import "monaco-editor/editor/contrib/snippet/browser/snippetController2.js"
import "monaco-editor/editor/contrib/suggest/browser/suggestController.js"
import "monaco-editor/editor/contrib/tokenization/browser/tokenization.js"
import "monaco-editor/editor/contrib/wordHighlighter/browser/wordHighlighter.js"
import "monaco-editor/editor/contrib/wordOperations/browser/wordOperations.js"
import "monaco-editor/editor/common/standaloneStrings.js"
// The package exports scripts only; this one brings the icon font's styles.
import "monaco-editor/features/codicon/register.js"
import { jsonDefaults } from "monaco-editor/languages/features/json/register.js"
import "monaco-editor/languages/definitions/register.all.js"
import EditorWorker from "monaco-editor/editor/editor.worker.js?worker"
import JsonWorker from "monaco-editor/languages/features/json/json.worker.js?worker"

import {
  BRACKET_COLORS,
  diagnosticsOptions,
  hexColor,
  MORE_FILES,
  tokenRules,
} from "@/lib/code-editor"
import type { JsonSchema } from "@/lib/schema-form"
import {
  scriptCompletions,
  type CompletionKind,
  type ScriptContext,
} from "@/lib/script-completions"

// Monaco asks for its workers here before any other way, which would need blob or remote URLs.
// Registering a language Monaco has again adds these names to the ones it already goes by.
for (const files of MORE_FILES) monaco.languages.register(files)

globalThis.MonacoEnvironment = {
  getWorker: (_module: string, label: string) =>
    label === "json" ? new JsonWorker() : new EditorWorker(),
}

export const THEME = "thoric"

/** A rendered color as `#RRGGBB[AA]`: Monaco takes no oklch, which the interface's tokens use. */
function rendered(color: string, canvas: CanvasRenderingContext2D) {
  canvas.clearRect(0, 0, 1, 1)
  canvas.fillStyle = "#000"
  canvas.fillStyle = color
  canvas.fillRect(0, 0, 1, 1)
  return hexColor([...canvas.getImageData(0, 0, 1, 1).data])
}

/** The theme from the interface's current colors; the editor's own background stays clear. */
function applyTheme() {
  const style = getComputedStyle(document.documentElement)
  const canvas = document.createElement("canvas").getContext("2d", { willReadFrequently: true })
  if (!canvas) return
  const color = (variable: string) =>
    rendered(style.getPropertyValue(variable).trim() || "#808080", canvas)
  const mixed = (variable: string, percent: number) =>
    rendered(
      `color-mix(in oklab, ${style.getPropertyValue(variable).trim()} ${percent}%, transparent)`,
      canvas
    )
  const dark = document.documentElement.classList.contains("dark")
  monaco.editor.defineTheme(THEME, {
    base: dark ? "vs-dark" : "vs",
    inherit: true,
    rules: tokenRules(color),
    colors: {
      "editor.background": "#00000000",
      "editor.foreground": color("--foreground"),
      "editorCursor.foreground": color("--foreground"),
      "editorLineNumber.foreground": color("--muted-foreground"),
      "editorLineNumber.activeForeground": color("--foreground"),
      "editor.lineHighlightBackground": "#00000000",
      "editor.lineHighlightBorder": "#00000000",
      "editor.selectionBackground": mixed("--primary", 25),
      "editor.inactiveSelectionBackground": mixed("--primary", 15),
      "editorGutter.background": "#00000000",
      ...Object.fromEntries(
        BRACKET_COLORS.map((variable, index) => [
          `editorBracketHighlight.foreground${index + 1}`,
          color(variable),
        ])
      ),
      "editorWidget.background": color("--popover"),
      "editorWidget.foreground": color("--popover-foreground"),
      "editorWidget.border": color("--border"),
      // Suggestions and menus mark a row as the interface's menus do and keep its text the
      // popover's: the light base theme would write the chosen row white, for a blue of its own.
      "editorSuggestWidget.foreground": color("--popover-foreground"),
      "editorSuggestWidget.selectedBackground": mixed("--foreground", 10),
      "editorSuggestWidget.selectedForeground": color("--popover-foreground"),
      "editorSuggestWidget.selectedIconForeground": color("--popover-foreground"),
      "editorSuggestWidget.highlightForeground": color("--code-command"),
      "editorSuggestWidget.focusHighlightForeground": color("--code-command"),
      "editorSuggestWidgetStatus.foreground": color("--muted-foreground"),
      "list.activeSelectionBackground": mixed("--foreground", 10),
      "list.activeSelectionForeground": color("--popover-foreground"),
      "list.activeSelectionIconForeground": color("--popover-foreground"),
      "list.inactiveSelectionBackground": mixed("--foreground", 10),
      "list.inactiveSelectionForeground": color("--popover-foreground"),
      "list.hoverBackground": mixed("--foreground", 5),
      "list.hoverForeground": color("--popover-foreground"),
      "list.highlightForeground": color("--code-command"),
      "list.focusHighlightForeground": color("--code-command"),
      "list.focusOutline": "#00000000",
      "menu.background": color("--popover"),
      "menu.foreground": color("--popover-foreground"),
      "menu.selectionBackground": mixed("--foreground", 10),
      "menu.selectionForeground": color("--popover-foreground"),
      "menu.border": color("--border"),
      "menu.separatorBackground": color("--border"),
      "editorHoverWidget.background": color("--popover"),
      "editorHoverWidget.foreground": color("--popover-foreground"),
      "editorHoverWidget.border": color("--border"),
      // A completion's documentation: its links and code.
      "textLink.foreground": color("--code-command"),
      "textLink.activeForeground": color("--code-command"),
      "textPreformat.foreground": color("--foreground"),
      "textPreformat.background": mixed("--foreground", 8),
      "textCodeBlock.background": mixed("--foreground", 5),
      focusBorder: "#00000000",
      "scrollbar.shadow": "#00000000",
    },
  })
  monaco.editor.setTheme(THEME)
}
applyTheme()
// The theme provider sets the class on the root; the editors follow when it changes.
new MutationObserver(applyTheme).observe(document.documentElement, {
  attributes: true,
  attributeFilter: ["class"],
})
// The mono face measures differently before it has loaded.
void document.fonts?.ready.then(() => monaco.editor.remeasureFonts())

const schemas = new Map<string, JsonSchema>()

/** Check the model at `model` against `schema`, or nothing when it has none. */
export function setSchema(model: string, schema: JsonSchema | undefined) {
  if (schema) schemas.set(model, schema)
  else schemas.delete(model)
  jsonDefaults.setDiagnosticsOptions(diagnosticsOptions(schemas))
}

/** Each script editor's context, by its model's address as Monaco writes it. */
const scripts = new Map<string, ScriptContext>()

/** Complete the shell script at `model` with what Thoric gives a script of `context`. */
export function setScriptContext(model: string, context: ScriptContext | undefined) {
  const key = monaco.Uri.parse(model).toString()
  if (context) scripts.set(key, context)
  else scripts.delete(key)
}

const { CompletionItemKind: Kind } = monaco.languages
const KINDS: Record<CompletionKind, monaco.languages.CompletionItemKind> = {
  parameter: Kind.Variable,
  variable: Kind.Constant,
  command: Kind.Function,
  subcommand: Kind.Method,
  snippet: Kind.Snippet,
  word: Kind.Text,
}

// Monaco asks the providers of a language before its word completion and stops at the first
// that answers, so a script with a context gets its own words from here too. One without, such
// as a shell file of a volume, keeps Monaco's words alone.
monaco.languages.registerCompletionItemProvider("shell", {
  triggerCharacters: ["$", "{"],
  provideCompletionItems(model, position, context) {
    const script = scripts.get(model.uri.toString())
    if (!script) return undefined
    const line = model.getLineContent(position.lineNumber)
    const cursor = position.column - 1
    const items = scriptCompletions({
      before: line.slice(0, cursor),
      after: line.slice(cursor),
      text: model.getValue(),
      context: script,
      triggered: context.triggerKind === monaco.languages.CompletionTriggerKind.TriggerCharacter,
    })
    return {
      suggestions: items.map((item) => ({
        label: item.label,
        kind: KINDS[item.kind],
        insertText: item.insert,
        insertTextRules: item.snippet
          ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
          : undefined,
        detail: item.detail,
        documentation: item.documentation ? { value: item.documentation } : undefined,
        filterText: item.filter,
        sortText: item.sort,
        range: {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: item.start + 1,
          endColumn: item.end + 1,
        },
      })),
    }
  },
})

export { monaco }
