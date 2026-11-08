import * as React from "react"
import { RiDeleteBack2Line, RiPlayLine, RiStopLine } from "@remixicon/react"

import { CodeEditor, CodeReader } from "@/components/code-editor"
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { t } from "@/lib/i18n"
import {
  inputSchemaOf,
  itemKey,
  itemName,
  itemTitle,
  toolHints,
  type Item,
  type ItemKind,
  type ResourceItem,
  type ToolItem,
} from "@/lib/inspector"
import {
  formDraft,
  formFields,
  freeForm,
  isPlainObject,
  normalizeSchema,
  prettyJson,
  readForm,
  type Draft,
  type FieldChoices,
  type FieldError,
} from "@/lib/schema-form"
import { fileSize } from "@/lib/uploads"
import { ResultView, type RunState } from "./result-view"
import { SchemaForm } from "./schema-fields"

type Mode = "form" | "json"

/** One tool, prompt, resource or template: what it is, its arguments, its last result. */
export function ItemDetail({
  kind,
  item,
  gone,
  run,
  lastArgs,
  ready,
  choices,
  onRun,
  onCancel,
}: {
  kind: ItemKind
  item: Item
  /** The server no longer lists it. */
  gone: boolean
  run?: RunState
  lastArgs?: Record<string, unknown>
  /** Whether the server can be asked now. */
  ready: boolean
  /** What a text field offers to fill in beside typing, by its name. */
  choices?: ReadonlyMap<string, FieldChoices>
  onRun(args: Record<string, unknown>): void
  onCancel(): void
}) {
  const schema = inputSchemaOf(kind, item)
  const hasFields = formFields(schema) > 0
  const jsonOnly = !hasFields && freeForm(normalizeSchema(schema, schema))
  const [mode, setMode] = React.useState<Mode>(jsonOnly ? "json" : "form")
  const [fields, setFields] = React.useState<Record<string, Draft>>(() =>
    formDraft(schema, lastArgs)
  )
  const [json, setJson] = React.useState(() => prettyJson(lastArgs ?? {}))
  const [errors, setErrors] = React.useState<FieldError[]>([])
  const [problem, setProblem] = React.useState<string>()
  const errorMap = React.useMemo(
    () => new Map(errors.map((error) => [error.path, error.message])),
    [errors]
  )
  const clear = React.useCallback(
    (path: string) => setErrors((current) => current.filter((error) => error.path !== path)),
    []
  )
  const running = run?.status === "running"
  const key = itemKey(kind, item)

  /** The arguments the open editor holds, or why they cannot be sent. */
  const read = (): Record<string, unknown> | null => {
    if (kind === "resource") return {}
    if (mode === "json") {
      try {
        const value: unknown = JSON.parse(json.trim() || "{}")
        if (!isPlainObject(value)) throw new Error(t("inspector.args.object"))
        return value
      } catch (error) {
        setProblem(t("inspector.args.invalid", { error: (error as Error).message }))
        return null
      }
    }
    const { value, errors: found } = readForm(schema, fields)
    setErrors(found)
    if (found.length) {
      setProblem(
        t("inspector.args.fix", {
          fields: found.map((error) => `${error.path}: ${error.message}`).join("; "),
        })
      )
      return null
    }
    return value
  }

  const submit = () => {
    if (!ready || running || gone) return
    const args = read()
    if (!args) return
    setProblem(undefined)
    onRun(args)
  }

  const switchMode = (next: Mode) => {
    if (next === mode) return
    if (next === "json") setJson(prettyJson(readForm(schema, fields).value))
    else
      try {
        const value: unknown = JSON.parse(json.trim() || "{}")
        if (!isPlainObject(value)) throw new Error(t("inspector.args.object"))
        setFields(formDraft(schema, value))
      } catch (error) {
        setProblem(t("inspector.args.noForm", { error: (error as Error).message }))
        return
      }
    setErrors([])
    setProblem(undefined)
    setMode(next)
  }

  const reset = () => {
    setFields(formDraft(schema))
    setJson("{}")
    setErrors([])
    setProblem(undefined)
  }

  const heading =
    kind === "template"
      ? t("inspector.args.variables")
      : kind === "tool"
        ? t("inspector.args.parameters")
        : t("inspector.args.arguments")
  const runLabel =
    kind === "tool"
      ? t("inspector.run.tool")
      : kind === "prompt"
        ? t("inspector.run.prompt")
        : t("inspector.run.resource")

  return (
    <div
      className="flex min-w-0 flex-col gap-5"
      onKeyDown={(event) => {
        if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
          event.preventDefault()
          submit()
        } else if (event.key === "Escape" && running) {
          event.preventDefault()
          onCancel()
        }
      }}
    >
      <Header kind={kind} item={item} />
      {gone && (
        <Alert>
          <AlertTitle>{t("inspector.item.gone")}</AlertTitle>
          <AlertDescription>{t("inspector.item.goneHint")}</AlertDescription>
        </Alert>
      )}
      {kind !== "resource" && (
        <section className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h3 className="text-sm font-medium">{heading}</h3>
            {(hasFields || kind === "tool") && (
              <ToggleGroup
                type="single"
                variant="outline"
                size="sm"
                spacing={0}
                value={mode}
                onValueChange={(value) => value && switchMode(value as Mode)}
                aria-label={t("inspector.args.mode")}
              >
                <ToggleGroupItem value="form">{t("inspector.args.form")}</ToggleGroupItem>
                <ToggleGroupItem value="json">JSON</ToggleGroupItem>
              </ToggleGroup>
            )}
          </div>
          {mode === "json" ? (
            <div className="flex flex-col gap-1.5">
              <CodeEditor
                language="json"
                value={json}
                onChange={setJson}
                onSubmit={submit}
                schema={schema}
                label={t("inspector.args.json")}
                placeholder="{ }"
                minHeight={128}
                maxHeight={480}
              />
              <p className="text-xs text-muted-foreground">{t("inspector.args.jsonHint")}</p>
            </div>
          ) : hasFields ? (
            <SchemaForm
              schema={schema}
              fields={fields}
              onChange={setFields}
              errors={errorMap}
              clear={clear}
              submit={submit}
              choices={choices}
            />
          ) : (
            <p className="text-xs text-muted-foreground">
              {kind === "tool" ? t("inspector.args.noParameters") : t("inspector.args.none")}
            </p>
          )}
        </section>
      )}
      {problem && (
        <p role="alert" className="text-xs break-words text-destructive">
          {problem}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" onClick={submit} disabled={!ready || running || gone}>
          <RiPlayLine data-icon="inline-start" />
          {runLabel}
        </Button>
        {running && (
          <Button type="button" variant="outline" onClick={onCancel}>
            <RiStopLine data-icon="inline-start" />
            {t("common.cancel")}
          </Button>
        )}
        {kind !== "resource" && (
          <Button type="button" variant="ghost" onClick={reset}>
            <RiDeleteBack2Line data-icon="inline-start" />
            {t("inspector.args.clear")}
          </Button>
        )}
        <span className="ml-auto hidden text-xs text-muted-foreground sm:inline">
          {running ? t("inspector.run.cancelKeys") : t("inspector.run.keys")}
        </span>
      </div>
      {run && <ResultView kind={kind} itemKey={key} run={run} />}
      <Definition kind={kind} item={item} />
    </div>
  )
}

const KIND_LABELS = {
  tool: "inspector.kind.tool",
  prompt: "inspector.kind.prompt",
  resource: "inspector.kind.resource",
  template: "inspector.kind.template",
} as const

const HINT_LABELS = {
  readOnly: "inspector.hint.readOnly",
  destructive: "inspector.hint.destructive",
  idempotent: "inspector.hint.idempotent",
  openWorld: "inspector.hint.openWorld",
  structured: "inspector.hint.structured",
} as const

function Header({ kind, item }: { kind: ItemKind; item: Item }) {
  const title = itemTitle(kind, item)
  const resource = item as ResourceItem
  const hints = kind === "tool" ? toolHints(item as ToolItem) : []
  return (
    <header className="flex min-w-0 flex-col gap-2">
      <Badge variant="secondary">{t(KIND_LABELS[kind])}</Badge>
      <h2 className="font-mono text-base font-medium break-all">{itemName(kind, item)}</h2>
      {title && <p className="text-sm text-muted-foreground">{title}</p>}
      {(kind === "resource" || kind === "template") && (
        <code className="font-mono text-xs break-all text-muted-foreground">
          {itemKey(kind, item)}
        </code>
      )}
      {(hints.length > 0 || resource.mimeType || typeof resource.size === "number") && (
        <div className="flex flex-wrap gap-1.5">
          {hints.map((hint) => (
            <Badge
              key={hint}
              variant="outline"
              className={hint === "destructive" ? "text-warning" : "text-muted-foreground"}
            >
              {t(HINT_LABELS[hint as keyof typeof HINT_LABELS])}
            </Badge>
          ))}
          {resource.mimeType && (
            <Badge variant="outline" className="font-mono text-muted-foreground">
              {resource.mimeType}
            </Badge>
          )}
          {typeof resource.size === "number" && (
            <Badge variant="outline" className="text-muted-foreground">
              {fileSize(resource.size)}
            </Badge>
          )}
        </div>
      )}
      {item.description && (
        <p className="text-sm break-words whitespace-pre-wrap">{item.description}</p>
      )}
    </header>
  )
}

/** The item as the server defines it: its schemas and the whole of its list entry. */
function Definition({ kind, item }: { kind: ItemKind; item: Item }) {
  const tool = item as ToolItem
  const parts = [
    kind === "tool" && {
      value: "input",
      label: t("inspector.definition.input"),
      text: prettyJson(tool.inputSchema ?? {}),
    },
    kind === "tool" &&
      tool.outputSchema && {
        value: "output",
        label: t("inspector.definition.output"),
        text: prettyJson(tool.outputSchema),
      },
    { value: "all", label: t("inspector.definition.all"), text: prettyJson(item) },
  ].filter((part): part is { value: string; label: string; text: string } => !!part)
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">{t("inspector.definition.title")}</h3>
      <Accordion type="multiple" className="rounded-xl border px-3">
        {parts.map((part) => (
          <AccordionItem key={part.value} value={part.value}>
            <AccordionTrigger className="py-2 text-xs">{part.label}</AccordionTrigger>
            <AccordionContent className="pt-0 pb-3">
              <CodeReader language="json" value={part.text} label={part.label} />
            </AccordionContent>
          </AccordionItem>
        ))}
      </Accordion>
    </section>
  )
}
