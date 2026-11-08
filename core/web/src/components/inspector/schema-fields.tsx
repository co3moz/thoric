import * as React from "react"
import { RiAddLine, RiArrowDownLine, RiArrowUpLine, RiCloseLine } from "@remixicon/react"
import { cn } from "cn"

import { CodeEditor } from "@/components/code-editor"
import { ConceptIcon } from "@/components/concept"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { t } from "@/lib/i18n"
import { editorSchema } from "@/lib/code-editor"
import {
  childPath,
  isPlainObject,
  constraintHint,
  discriminator,
  draftOf,
  enumValues,
  fieldKind,
  fieldName,
  hasDefault,
  newItem,
  normalizeSchema,
  objectVariants,
  switchVariant,
  typeLabel,
  type ArrayItem,
  type Draft,
  type FieldChoices,
  type JsonSchema,
} from "@/lib/schema-form"

const NO_CHOICES: ReadonlyMap<string, FieldChoices> = new Map()

/**
 * What every field of one form shares: the root schema for `$ref`s, the errors of the last run,
 * the run itself, which a JSON field's editor takes Ctrl+Enter for, and the values text fields
 * offer by their name.
 */
const FormContext = React.createContext<{
  root: JsonSchema
  errors: ReadonlyMap<string, string>
  clear(path: string): void
  submit?(): void
  choices: ReadonlyMap<string, FieldChoices>
}>({ root: {}, errors: new Map(), clear: () => {}, choices: NO_CHOICES })

/** Radix selects take no empty value, so a field left unset is this one. */
const UNSET = "unset"

type FieldProps = {
  name: string
  schema: unknown
  depth: number
  required: boolean
  draft: Draft | undefined
  onChange(draft: Draft): void
  path: string
  /** An array item: no label of its own. */
  bare?: boolean
}

/** The fields of a root object schema, such as a tool's inputSchema. */
export function SchemaForm({
  schema,
  fields,
  onChange,
  errors,
  clear,
  submit,
  choices = NO_CHOICES,
}: {
  schema: JsonSchema
  fields: Record<string, Draft>
  onChange(fields: Record<string, Draft>): void
  errors: ReadonlyMap<string, string>
  clear(path: string): void
  submit?(): void
  /** What a text field offers to fill in beside typing, by its name. */
  choices?: ReadonlyMap<string, FieldChoices>
}) {
  const context = React.useMemo(
    () => ({ root: schema, errors, clear, submit, choices }),
    [schema, errors, clear, submit, choices]
  )
  return (
    <FormContext.Provider value={context}>
      <ObjectFields
        schema={normalizeSchema(schema, schema)}
        depth={0}
        fields={fields}
        onChange={onChange}
        path=""
      />
    </FormContext.Provider>
  )
}

function ObjectFields({
  schema,
  depth,
  fields,
  onChange,
  path,
  skip,
}: {
  schema: JsonSchema
  depth: number
  fields: Record<string, Draft>
  onChange(fields: Record<string, Draft>): void
  path: string
  skip?: string
}) {
  const required = new Set(schema.required ?? [])
  const entries = Object.entries(schema.properties ?? {}).filter(([name]) => name !== skip)
  if (!entries.length)
    return <p className="text-xs text-muted-foreground">{t("inspector.form.noOtherFields")}</p>
  return (
    <div className="flex flex-col gap-4">
      {entries.map(([name, property]) => (
        <SchemaField
          key={name}
          name={name}
          schema={property}
          depth={depth}
          required={required.has(name)}
          draft={fields[name]}
          onChange={(draft) => onChange({ ...fields, [name]: draft })}
          path={childPath(path, name)}
        />
      ))}
    </div>
  )
}

function SchemaField(props: FieldProps) {
  const { root } = React.useContext(FormContext)
  const schema = normalizeSchema(props.schema, root)
  const kind = fieldKind(schema, root, props.depth)
  const fresh = draftOf(props.schema, root, props.depth, undefined, props.required)
  // A draft of another kind, such as after the server changed the schema, starts anew.
  const draft = props.draft?.kind === fresh.kind ? props.draft : fresh
  const field = { ...props, schema, draft }
  switch (kind) {
    case "enum":
      return <EnumField {...field} />
    case "union":
      return <UnionField {...field} />
    case "boolean":
      return <BooleanField {...field} />
    case "number":
    case "string":
      return <TextField {...field} number={kind === "number"} />
    case "object":
      return <ObjectField {...field} />
    case "array":
      return <ArrayField {...field} />
    case "json":
      return <JsonField {...field} raw={props.schema} />
  }
}

type Resolved = Omit<FieldProps, "schema"> & { schema: JsonSchema; draft: Draft }

/** The label, description, limits and error around a field's control. */
function Shell({
  id,
  name,
  schema,
  required,
  bare,
  error,
  children,
}: {
  id: string
  name: string
  schema: JsonSchema
  required: boolean
  bare?: boolean
  error?: string
  children: React.ReactNode
}) {
  const { root } = React.useContext(FormContext)
  const hint = constraintHint(schema)
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      {!bare && (
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <Label htmlFor={id} className="font-mono text-xs break-all">
            {fieldName(name, schema)}
            {required && (
              <>
                <span aria-hidden className="text-destructive">
                  *
                </span>
                <span className="sr-only">{t("inspector.form.requiredLabel")}</span>
              </>
            )}
          </Label>
          <TypeTag>{typeLabel(schema, root)}</TypeTag>
        </div>
      )}
      {!bare && schema.description && <Description>{schema.description}</Description>}
      {children}
      {hint && <p className="text-xs break-words text-muted-foreground">{hint}</p>}
      {error && (
        <p id={`${id}-error`} className="text-xs text-destructive">
          {error}
        </p>
      )}
    </div>
  )
}

const TypeTag = ({ children }: { children: React.ReactNode }) => (
  <Badge variant="outline" className="h-4 px-1.5 font-mono text-[0.65rem] text-muted-foreground">
    {children}
  </Badge>
)

const Description = ({ children }: { children: React.ReactNode }) => (
  <p className="text-xs break-words whitespace-pre-wrap text-muted-foreground">{children}</p>
)

/** A field's id, its error from the last run, and how to clear it once it changes. */
function useField(path: string) {
  const id = React.useId()
  const { errors, clear } = React.useContext(FormContext)
  const error = errors.get(path)
  return {
    id,
    error,
    edited: () => {
      if (error) clear(path)
    },
    invalid: error
      ? { "aria-invalid": true as const, "aria-describedby": `${id}-error` }
      : { "aria-invalid": undefined },
  }
}

function placeholder(schema: JsonSchema) {
  if (hasDefault(schema)) return String(schema.default)
  const example = schema.examples?.[0]
  return example === undefined
    ? ""
    : t("inspector.form.example", {
        value: typeof example === "string" ? example : JSON.stringify(example),
      })
}

function TextField({ number, ...field }: Resolved & { number: boolean }) {
  const { id, error, edited, invalid } = useField(field.path)
  const { choices } = React.useContext(FormContext)
  const offered = number || field.bare ? undefined : choices.get(field.name)
  const text = (field.draft as Extract<Draft, { kind: "text" }>).text
  const change = (value: string) => {
    edited()
    field.onChange({ kind: "text", text: value })
  }
  const area = (className?: string) => (
    <Textarea
      id={id}
      rows={1}
      spellCheck={false}
      value={text}
      placeholder={placeholder(field.schema)}
      aria-label={field.bare ? field.name : undefined}
      className={cn("min-h-9 py-1.5", className)}
      onChange={(event) => change(event.target.value)}
      {...invalid}
    />
  )
  return (
    <Shell id={id} {...field} error={error}>
      {offered ? (
        <div className="flex min-w-0 flex-col gap-2 sm:flex-row sm:items-start">
          {area("sm:flex-1")}
          <ChoicePicker choices={offered} text={text} onChoose={change} />
        </div>
      ) : number ? (
        <Input
          id={id}
          type="number"
          inputMode="decimal"
          step={field.schema.type === "integer" ? 1 : "any"}
          min={field.schema.minimum}
          max={field.schema.maximum}
          value={text}
          placeholder={placeholder(field.schema)}
          aria-label={field.bare ? field.name : undefined}
          onChange={(event) => change(event.target.value)}
          {...invalid}
        />
      ) : (
        area()
      )}
    </Shell>
  )
}

/** The values a text field offers: choosing one fills the field in, which stays free text. */
function ChoicePicker({
  choices,
  text,
  onChoose,
}: {
  choices: FieldChoices
  text: string
  onChoose(value: string): void
}) {
  const typed = text.trim()
  const chosen = choices.items.some((item) => item.value === typed) ? typed : ""
  const none = !choices.items.length
  return (
    <Select value={chosen} onValueChange={onChoose} disabled={none}>
      <SelectTrigger className="w-full sm:w-56" aria-label={choices.label}>
        <SelectValue placeholder={none ? choices.empty : choices.label} />
      </SelectTrigger>
      <SelectContent>
        <SelectGroup>
          {choices.items.map((item) => (
            <SelectItem key={item.value} value={item.value}>
              {choices.concept && <ConceptIcon concept={choices.concept} />}
              <span className="truncate">{item.label}</span>
              {item.detail && <span className="text-muted-foreground">{item.detail}</span>}
            </SelectItem>
          ))}
        </SelectGroup>
      </SelectContent>
    </Select>
  )
}

function BooleanField(field: Resolved) {
  const { id, error } = useField(field.path)
  const value = (field.draft as Extract<Draft, { kind: "boolean" }>).value
  if (field.required)
    return (
      <Shell id={id} {...field} error={error}>
        <div className="flex items-center gap-2">
          <Switch
            id={id}
            checked={value === true}
            aria-label={field.bare ? field.name : undefined}
            onCheckedChange={(checked) => field.onChange({ kind: "boolean", value: checked })}
          />
          <span className="font-mono text-xs text-muted-foreground">
            {value === true ? "true" : "false"}
          </span>
        </div>
      </Shell>
    )
  return (
    <Shell id={id} {...field} error={error}>
      <Select
        value={value === undefined ? UNSET : String(value)}
        onValueChange={(chosen) =>
          field.onChange({
            kind: "boolean",
            value: chosen === UNSET ? undefined : chosen === "true",
          })
        }
      >
        <SelectTrigger
          id={id}
          className="w-full sm:w-64"
          aria-label={field.bare ? field.name : undefined}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value={UNSET}>{unsetLabel(field.schema)}</SelectItem>
            <SelectItem value="true">true</SelectItem>
            <SelectItem value="false">false</SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>
    </Shell>
  )
}

const unsetLabel = (schema: JsonSchema) =>
  hasDefault(schema)
    ? t("inspector.form.unsetDefault", { value: JSON.stringify(schema.default) })
    : t("inspector.form.unset")

const valueLabel = (value: unknown) => (typeof value === "string" ? value : JSON.stringify(value))

function EnumField(field: Resolved) {
  const { id, error, edited, invalid } = useField(field.path)
  const values = enumValues(field.schema) ?? []
  const index = (field.draft as Extract<Draft, { kind: "enum" }>).index
  const fixed = field.required && values.length === 1
  return (
    <Shell id={id} {...field} error={error}>
      <Select
        value={index === undefined ? UNSET : String(index)}
        onValueChange={(chosen) => {
          edited()
          field.onChange({ kind: "enum", index: chosen === UNSET ? undefined : Number(chosen) })
        }}
      >
        <SelectTrigger
          id={id}
          className="w-full sm:w-64"
          aria-label={field.bare ? field.name : undefined}
          {...invalid}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            {!fixed && <SelectItem value={UNSET}>{unsetLabel(field.schema)}</SelectItem>}
            {values.map((value, position) => (
              <SelectItem key={position} value={String(position)} className="font-mono text-xs">
                {valueLabel(value)}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
    </Shell>
  )
}

/** A frame around the fields of a nested object, or nothing around an array item's. */
function Group({
  name,
  schema,
  required,
  bare,
  tag,
  error,
  children,
}: {
  name: string
  schema: JsonSchema
  required: boolean
  bare?: boolean
  tag: string
  error?: string
  children: React.ReactNode
}) {
  if (bare)
    return (
      <div className="flex flex-col gap-3">
        {children}
        {error && <p className="text-xs text-destructive">{error}</p>}
      </div>
    )
  return (
    <fieldset className="flex min-w-0 flex-col gap-3 rounded-xl border px-3 pt-1 pb-3">
      <legend className="flex flex-wrap items-center gap-x-2 px-1 font-mono text-xs font-medium">
        <span className="break-all">{fieldName(name, schema)}</span>
        {required && (
          <>
            <span aria-hidden className="text-destructive">
              *
            </span>
            <span className="sr-only">{t("inspector.form.requiredLabel")}</span>
          </>
        )}
        <TypeTag>{tag}</TypeTag>
      </legend>
      {schema.description && <Description>{schema.description}</Description>}
      {children}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </fieldset>
  )
}

function ObjectField(field: Resolved) {
  const { error } = useField(field.path)
  const draft = field.draft as Extract<Draft, { kind: "object" }>
  return (
    <Group {...field} tag="object" error={error}>
      <ObjectFields
        schema={field.schema}
        depth={field.depth + 1}
        fields={draft.fields}
        onChange={(fields) => field.onChange({ kind: "object", fields })}
        path={field.path}
      />
    </Group>
  )
}

/** A `oneOf` or `anyOf` of objects: which one, by its discriminator, and that one's fields. */
function UnionField(field: Resolved) {
  const { root } = React.useContext(FormContext)
  const { id, error, edited } = useField(field.path)
  const draft = field.draft as Extract<Draft, { kind: "union" }>
  const { variants, key } = objectVariants(field.schema, root)!
  const chosen = draft.variant === undefined ? undefined : variants[draft.variant]
  const label = (variant: JsonSchema, index: number) => {
    if (key) return valueLabel(discriminator(variant, key, root))
    return (
      variant.title ||
      variant.description?.split("\n")[0].slice(0, 60) ||
      t("inspector.form.option", { number: index + 1 })
    )
  }
  return (
    <Group
      {...field}
      tag={t("inspector.type.oneOfCount", { count: variants.length })}
      error={undefined}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Label htmlFor={id} className="font-mono text-xs text-muted-foreground">
          {key ?? t("inspector.form.shape")}
        </Label>
        <Select
          value={draft.variant === undefined ? UNSET : String(draft.variant)}
          onValueChange={(value) => {
            edited()
            field.onChange(
              switchVariant(
                field.schema,
                root,
                field.depth,
                draft,
                value === UNSET ? undefined : Number(value)
              )
            )
          }}
        >
          <SelectTrigger
            id={id}
            size="sm"
            className="w-full sm:w-64"
            aria-invalid={!!error || undefined}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              {!field.required && !field.bare && (
                <SelectItem value={UNSET}>{t("inspector.form.unset")}</SelectItem>
              )}
              {variants.map((variant, index) => (
                <SelectItem key={index} value={String(index)} className="font-mono text-xs">
                  {label(variant, index)}
                </SelectItem>
              ))}
            </SelectGroup>
          </SelectContent>
        </Select>
      </div>
      {chosen?.description && <Description>{chosen.description}</Description>}
      {error && <p className="text-xs text-destructive">{error}</p>}
      {chosen && (
        <ObjectFields
          schema={chosen}
          depth={field.depth + 1}
          fields={draft.fields}
          onChange={(fields) => field.onChange({ ...draft, fields })}
          path={field.path}
          skip={key}
        />
      )}
    </Group>
  )
}

function ArrayField(field: Resolved) {
  const { root } = React.useContext(FormContext)
  const { error, edited } = useField(field.path)
  const items = (field.draft as Extract<Draft, { kind: "array" }>).items
  const itemSchema = normalizeSchema(field.schema.items, root)
  const union = objectVariants(itemSchema, root)
  // Plain values get a row each; objects and unions a frame of their own.
  const compact = !union && itemSchema.type !== "object" && itemSchema.type !== "array"
  const set = (next: ArrayItem[]) => {
    edited()
    field.onChange({ kind: "array", items: next })
  }
  const move = (from: number, to: number) => {
    const next = [...items]
    ;[next[from], next[to]] = [next[to], next[from]]
    set(next)
  }
  const add = (variant?: number) =>
    set([...items, newItem(field.schema, root, field.depth, variant)])
  const hint = constraintHint(field.schema)
  return (
    <Group {...field} tag={`${typeLabel(field.schema, root)} · ${items.length}`} error={error}>
      {items.length ? (
        <ol className="flex flex-col gap-2">
          {items.map((item, index) => {
            const buttons = (
              <div className="flex shrink-0 items-center">
                <ItemButton
                  label={t("inspector.form.moveUp", { number: index + 1 })}
                  disabled={index === 0}
                  onClick={() => move(index, index - 1)}
                >
                  <RiArrowUpLine />
                </ItemButton>
                <ItemButton
                  label={t("inspector.form.moveDown", { number: index + 1 })}
                  disabled={index === items.length - 1}
                  onClick={() => move(index, index + 1)}
                >
                  <RiArrowDownLine />
                </ItemButton>
                <ItemButton
                  label={t("inspector.form.remove", { number: index + 1 })}
                  onClick={() => set(items.filter((other) => other.id !== item.id))}
                >
                  <RiCloseLine />
                </ItemButton>
              </div>
            )
            const inner = (
              <SchemaField
                name={`${field.name}[${index}]`}
                schema={field.schema.items}
                depth={field.depth + 1}
                required
                bare
                draft={item.draft}
                onChange={(draft) =>
                  set(items.map((other) => (other.id === item.id ? { ...other, draft } : other)))
                }
                path={childPath(field.path, index)}
              />
            )
            return (
              <li
                key={item.id}
                className={cn(
                  "flex min-w-0 gap-2",
                  compact ? "items-start" : "flex-col rounded-xl border bg-muted/20 p-2"
                )}
              >
                {compact ? (
                  <>
                    <span className="w-6 shrink-0 pt-2 text-right font-mono text-xs text-muted-foreground">
                      {index + 1}
                    </span>
                    <div className="min-w-0 flex-1">{inner}</div>
                    {buttons}
                  </>
                ) : (
                  <>
                    <div className="flex items-center justify-between gap-2">
                      <span className="font-mono text-xs text-muted-foreground">#{index + 1}</span>
                      {buttons}
                    </div>
                    {inner}
                  </>
                )}
              </li>
            )
          })}
        </ol>
      ) : (
        <p className="text-xs text-muted-foreground">{t("inspector.form.noItems")}</p>
      )}
      <div>
        {union?.key !== undefined ? (
          <Select value="" onValueChange={(value) => add(Number(value))}>
            <SelectTrigger
              size="sm"
              className="w-full sm:w-64"
              aria-label={t("inspector.form.addTo", { name: field.name })}
            >
              <SelectValue placeholder={t("inspector.form.addItem")} />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {union.variants.map((variant, index) => (
                  <SelectItem key={index} value={String(index)}>
                    <span className="font-mono text-xs">
                      {valueLabel(discriminator(variant, union.key!, root))}
                    </span>
                    {variant.description && (
                      <span className="truncate text-xs text-muted-foreground">
                        {variant.description.split("\n")[0].slice(0, 50)}
                      </span>
                    )}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        ) : (
          <Button type="button" variant="outline" size="sm" onClick={() => add()}>
            <RiAddLine data-icon="inline-start" />
            {t("inspector.form.addItem")}
          </Button>
        )}
      </div>
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </Group>
  )
}

function ItemButton({
  label,
  disabled,
  onClick,
  children,
}: {
  label: string
  disabled?: boolean
  onClick(): void
  children: React.ReactNode
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          disabled={disabled}
          aria-label={label}
          onClick={onClick}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

/**
 * A value the form cannot break down, such as a union of plain values, typed as JSON and checked
 * against the field's own schema, `$ref`s and all.
 */
function JsonField({ raw, ...field }: Resolved & { raw: unknown }) {
  const { root, submit } = React.useContext(FormContext)
  const { id, error, edited } = useField(field.path)
  const text = (field.draft as Extract<Draft, { kind: "text" }>).text
  return (
    <Shell id={id} {...field} error={error}>
      <CodeEditor
        language="json"
        value={text}
        schema={editorSchema(isPlainObject(raw) ? raw : {}, root)}
        label={field.name}
        minHeight={36}
        maxHeight={240}
        invalid={!!error}
        onSubmit={submit}
        placeholder={
          hasDefault(field.schema)
            ? JSON.stringify(field.schema.default)
            : field.schema.type === "array"
              ? "[ ]"
              : field.schema.type === "object"
                ? "{ }"
                : t("inspector.form.jsonValue")
        }
        onChange={(value) => {
          edited()
          field.onChange({ kind: "text", text: value })
        }}
      />
    </Shell>
  )
}
