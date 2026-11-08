// The inspector's argument form, made from a JSON Schema such as a tool's inputSchema. A schema
// gives each value a kind of field; what the form holds is a draft of the same shape, with the
// text people typed, and reading it gives the arguments and the fields that cannot be sent.
// The rules follow the VS Code MCP Server Inspector's form.
import type { Concept } from "./concepts"
import { t } from "./i18n"

/** A JSON Schema, of which the form reads what picks and checks a field. */
export type JsonSchema = {
  type?: string | string[]
  title?: string
  description?: string
  properties?: Record<string, JsonSchema>
  required?: string[]
  additionalProperties?: boolean | JsonSchema
  items?: JsonSchema | JsonSchema[]
  prefixItems?: JsonSchema[]
  enum?: unknown[]
  const?: unknown
  oneOf?: JsonSchema[]
  anyOf?: JsonSchema[]
  $ref?: string
  $schema?: string
  $defs?: Record<string, JsonSchema>
  definitions?: Record<string, JsonSchema>
  default?: unknown
  examples?: unknown[]
  format?: string
  pattern?: string
  minimum?: number
  maximum?: number
  exclusiveMinimum?: number
  exclusiveMaximum?: number
  minLength?: number
  maxLength?: number
  minItems?: number
  maxItems?: number
  /** Set by {@link normalizeSchema}: `null` was one of the types. */
  nullable?: boolean
  /** Set by {@link normalizeSchema}: the types besides `null`, when there were several. */
  typeList?: string[]
  [keyword: string]: unknown
}

/**
 * Values a text field offers beside typing, such as the containers a `container_id` names: one
 * fills the field in, and the text stays the field's to change.
 */
export type FieldChoices = {
  /** The picker's placeholder and accessible name. */
  label: string
  /** What the picker says when there is nothing to offer. */
  empty: string
  concept?: Concept
  items: { value: string; label: string; detail?: string }[]
}

/** Below this depth every value is a JSON field. */
export const MAX_DEPTH = 6
const REF_HOPS = 8

export const isPlainObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)

const asSchema = (value: unknown): JsonSchema => (isPlainObject(value) ? value : {})

/**
 * `object` without the keys named, which must be optional ones. It keeps the type, since `Omit`
 * of a type with an index signature, as a schema has, loses every named key.
 */
export function omit<T extends object>(object: T, ...keys: (keyof T)[]): T {
  const rest = { ...object }
  for (const key of keys) delete rest[key]
  return rest
}

/** Follow local `$ref`s (`#/$defs/...`), keeping the keywords beside a ref. */
export function resolveRef(schema: unknown, root: JsonSchema): JsonSchema {
  let current = asSchema(schema)
  for (let hop = 0; hop < REF_HOPS && current.$ref?.startsWith("#"); hop++) {
    const ref: string = current.$ref
    const target =
      ref === "#"
        ? root
        : ref
            .slice(2)
            .split("/")
            .reduce<unknown>(
              (node, part) =>
                isPlainObject(node)
                  ? node[decodeURIComponent(part).replace(/~1/g, "/").replace(/~0/g, "~")]
                  : undefined,
              root
            )
    if (!isPlainObject(target)) break
    current = { ...target, ...omit(current, "$ref") }
  }
  return current
}

/**
 * Resolve refs and simplify nullable types and unions, so the right field can be picked. A ref met
 * again inside its own union, as a recursive type such as a tree that may be null has, is taken
 * without its union, so the schema of a desktop server cannot recurse without end.
 */
export function normalizeSchema(
  schema: unknown,
  root: JsonSchema,
  within: readonly string[] = []
): JsonSchema {
  const ref = asSchema(schema).$ref
  let current = resolveRef(schema, root)
  if (typeof ref === "string" && within.includes(ref)) current = omit(current, "anyOf", "oneOf")
  let nullable = false
  if (Array.isArray(current.type)) {
    const types = current.type.filter((type) => type !== "null")
    nullable = types.length !== current.type.length
    current = { ...current, type: types.length === 1 ? types[0] : undefined, typeList: types }
  }
  const union = current.anyOf ?? current.oneOf
  if (Array.isArray(union)) {
    const path = typeof ref === "string" ? [...within, ref] : within
    const options = union.map((option) => normalizeSchema(option, root, path))
    const others = options.filter((option) => option.type !== "null")
    nullable ||= others.length !== options.length
    if (others.length === 1) {
      current = {
        ...others[0],
        ...omit(current, "anyOf", "oneOf"),
        description: current.description ?? others[0].description,
      }
    } else if (
      others.length &&
      others.every((option) => option.const !== undefined || Array.isArray(option.enum))
    ) {
      current = {
        ...current,
        enum: others.flatMap((option) =>
          option.const !== undefined ? [option.const] : (option.enum ?? [])
        ),
      }
    }
  }
  return nullable ? { ...current, nullable: true } : current
}

/** The one value a schema allows, through `const` or an `enum` of one. */
export function constValue(schema: JsonSchema): { value: unknown } | undefined {
  if ("const" in schema) return { value: schema.const }
  if (Array.isArray(schema.enum) && schema.enum.length === 1) return { value: schema.enum[0] }
  return undefined
}

export type Variants = {
  /** Each variant with the union's own properties merged in. */
  variants: JsonSchema[]
  /** The property whose constant tells the variants apart, such as `type`. */
  key?: string
}

/** The objects a `oneOf` or `anyOf` chooses between, or none when it is not a set of objects. */
export function objectVariants(schema: JsonSchema, root: JsonSchema): Variants | null {
  const union = schema.oneOf ?? schema.anyOf
  if (!Array.isArray(union)) return null
  const base = omit(schema, "oneOf", "anyOf")
  const variants = union
    .map((variant) => normalizeSchema(variant, root))
    .filter((variant) => variant.type !== "null")
    .map((variant) => ({
      ...base,
      ...variant,
      properties: { ...base.properties, ...variant.properties },
      required: [...new Set([...(base.required ?? []), ...(variant.required ?? [])])],
    }))
  if (
    variants.length < 2 ||
    !variants.every(
      (variant) =>
        (variant.type === "object" || variant.type === undefined) &&
        Object.keys(variant.properties).length
    )
  )
    return null
  const key = Object.keys(variants[0].properties).find((name) =>
    variants.every(
      (variant) =>
        variant.properties[name] &&
        constValue(normalizeSchema(variant.properties[name], root)) !== undefined
    )
  )
  return { variants, key }
}

/** What a variant's discriminator is set to. */
export const discriminator = (variant: JsonSchema, key: string, root: JsonSchema) =>
  constValue(normalizeSchema(variant.properties?.[key], root))?.value

/** Whether an array's items get fields of their own rather than one JSON field for the list. */
export function itemsRenderable(schema: JsonSchema, root: JsonSchema): boolean {
  if (!isPlainObject(schema.items) || Array.isArray(schema.prefixItems)) return false
  const items = normalizeSchema(schema.items, root)
  if (Array.isArray(items.enum) || items.const !== undefined || objectVariants(items, root))
    return true
  if (
    typeof items.type === "string" &&
    ["string", "number", "integer", "boolean"].includes(items.type)
  )
    return true
  return items.type === "object" && Object.keys(items.properties ?? {}).length > 0
}

export type FieldKind =
  "enum" | "union" | "boolean" | "number" | "string" | "object" | "array" | "json"

/** The field a normalized schema gets at `depth`. */
export function fieldKind(schema: JsonSchema, root: JsonSchema, depth: number): FieldKind {
  if (enumValues(schema)) return "enum"
  if (depth < MAX_DEPTH && objectVariants(schema, root)) return "union"
  if (schema.type === "boolean") return "boolean"
  if (schema.type === "number" || schema.type === "integer") return "number"
  if (schema.type === "string") return "string"
  if (schema.type === "object" && Object.keys(schema.properties ?? {}).length && depth < MAX_DEPTH)
    return "object"
  if (schema.type === "array" && depth < MAX_DEPTH && itemsRenderable(schema, root)) return "array"
  return "json"
}

/** The values an enum field offers: its `enum`, or its `const` alone. */
export function enumValues(schema: JsonSchema): unknown[] | null {
  if (Array.isArray(schema.enum)) return schema.enum
  return schema.const !== undefined ? [schema.const] : null
}

/** An object without declared properties that takes others: only JSON can say what it holds. */
export const freeForm = (schema: JsonSchema) =>
  !schema.properties &&
  (schema.additionalProperties === true || isPlainObject(schema.additionalProperties))

/** A default worth showing: a `null` default only says the value may be left out. */
export const hasDefault = (schema: JsonSchema) =>
  schema.default !== undefined && schema.default !== null

/** A field's name, with its title when it has a different one. */
export const fieldName = (name: string, schema: JsonSchema) =>
  schema.title && schema.title !== name ? `${schema.title} (${name})` : name

/** A short type, such as `string · uri`, `integer | null` or `string[]`. */
export function typeLabel(schema: JsonSchema, root: JsonSchema): string {
  if (Array.isArray(schema.enum)) return "enum"
  if (schema.type === "array") {
    const raw = isPlainObject(schema.items) ? schema.items : undefined
    if (raw?.$ref) return `${String(raw.$ref).split("/").pop()}[]`
    const items = raw && normalizeSchema(raw, root)
    const union = items && (items.oneOf ?? items.anyOf)
    if (Array.isArray(union)) return t("inspector.type.kinds", { count: union.length })
    return typeof items?.type === "string" ? `${items.type}[]` : "array"
  }
  if (schema.oneOf || schema.anyOf) return t("inspector.type.oneOf")
  if (typeof schema.type === "string") {
    const base = schema.format ? `${schema.type} · ${schema.format}` : schema.type
    return schema.nullable ? `${base} | null` : base
  }
  if (schema.typeList?.length) return schema.typeList.join(" | ")
  return t("inspector.type.any")
}

/** The limits a schema sets, as one line. */
export function constraintHint(schema: JsonSchema): string {
  const parts: string[] = []
  const add = (value: unknown, key: Parameters<typeof t>[0]) => {
    if (value !== undefined) parts.push(t(key, { value: String(value) }))
  }
  add(schema.minimum, "inspector.limit.minimum")
  add(schema.exclusiveMinimum, "inspector.limit.above")
  add(schema.maximum, "inspector.limit.maximum")
  add(schema.exclusiveMaximum, "inspector.limit.below")
  add(schema.minLength, "inspector.limit.minLength")
  add(schema.maxLength, "inspector.limit.maxLength")
  add(schema.minItems, "inspector.limit.minItems")
  add(schema.maxItems, "inspector.limit.maxItems")
  add(schema.pattern, "inspector.limit.pattern")
  if (hasDefault(schema)) add(JSON.stringify(schema.default), "inspector.limit.default")
  return parts.join(" · ")
}

/** What a field holds while it is edited. Numbers, strings and JSON keep the text typed. */
export type Draft =
  | { kind: "text"; text: string }
  | { kind: "boolean"; value?: boolean }
  | { kind: "enum"; index?: number }
  | { kind: "object"; fields: Record<string, Draft> }
  | { kind: "union"; variant?: number; fields: Record<string, Draft> }
  | { kind: "array"; items: ArrayItem[] }
export type ArrayItem = { id: number; draft: Draft }

let nextItem = 0
/** An array item, with an id of its own so a moved row keeps its field. */
export const arrayItem = (draft: Draft): ArrayItem => ({ id: ++nextItem, draft })

const draftKinds: Record<FieldKind, Draft["kind"]> = {
  enum: "enum",
  union: "union",
  boolean: "boolean",
  number: "text",
  string: "text",
  object: "object",
  array: "array",
  json: "text",
}

/** Whether a JSON field takes only JSON; any other text goes as a string. */
const strictJson = (schema: JsonSchema) =>
  ["array", "object", "number", "integer"].includes(String(schema.type))

export const prettyJson = (value: unknown) => JSON.stringify(value, null, 2) ?? ""

/** The variant a value looks like: by its discriminator, or by the most properties it shares. */
function variantOf(choice: Variants, value: unknown, root: JsonSchema): number | undefined {
  if (!isPlainObject(value)) return undefined
  const { variants, key } = choice
  if (key && key in value) {
    const index = variants.findIndex(
      (variant) => JSON.stringify(discriminator(variant, key, root)) === JSON.stringify(value[key])
    )
    if (index >= 0) return index
  }
  let best = 0
  let found: number | undefined
  variants.forEach((variant, index) => {
    const score = Object.keys(value).filter((name) => name in (variant.properties ?? {})).length
    if (score > best) {
      best = score
      found = index
    }
  })
  return found
}

/** The fields of an object's properties for `value`, leaving out `skip`. */
function objectDrafts(
  schema: JsonSchema,
  root: JsonSchema,
  depth: number,
  value: unknown,
  skip?: string
): Record<string, Draft> {
  const required = new Set(schema.required ?? [])
  const values = isPlainObject(value) ? value : {}
  return Object.fromEntries(
    Object.entries(schema.properties ?? {})
      .filter(([name]) => name !== skip)
      .map(([name, property]) => [
        name,
        draftOf(property, root, depth, values[name], required.has(name)),
      ])
  )
}

/** The draft that shows `value`, or an empty field without one. */
export function draftOf(
  raw: unknown,
  root: JsonSchema,
  depth: number,
  value: unknown,
  required: boolean
): Draft {
  const schema = normalizeSchema(raw, root)
  switch (fieldKind(schema, root, depth)) {
    case "enum": {
      const values = enumValues(schema)!
      const index = values.findIndex((option) => JSON.stringify(option) === JSON.stringify(value))
      return {
        kind: "enum",
        index: index >= 0 ? index : required && values.length === 1 ? 0 : undefined,
      }
    }
    case "union": {
      const choice = objectVariants(schema, root)!
      const variant = variantOf(choice, value, root) ?? (required ? 0 : undefined)
      return {
        kind: "union",
        variant,
        fields:
          variant === undefined
            ? {}
            : objectDrafts(choice.variants[variant], root, depth + 1, value, choice.key),
      }
    }
    case "boolean":
      return {
        kind: "boolean",
        value: typeof value === "boolean" ? value : required ? schema.default === true : undefined,
      }
    case "number":
      return {
        kind: "text",
        text: typeof value === "number" || typeof value === "string" ? String(value) : "",
      }
    case "string":
      return {
        kind: "text",
        text:
          typeof value === "string"
            ? value
            : value === undefined || value === null
              ? ""
              : JSON.stringify(value),
      }
    case "object":
      return { kind: "object", fields: objectDrafts(schema, root, depth + 1, value) }
    case "array":
      return {
        kind: "array",
        items: (Array.isArray(value) ? value : []).map((item) =>
          arrayItem(draftOf(schema.items, root, depth + 1, item, true))
        ),
      }
    case "json":
      return {
        kind: "text",
        text:
          value === undefined
            ? ""
            : typeof value === "string" && !strictJson(schema)
              ? value
              : prettyJson(value),
      }
  }
}

/** The draft of a new item in an array of `schema`'s items, of union variant `variant`. */
export function newItem(schema: JsonSchema, root: JsonSchema, depth: number, variant?: number) {
  const items = normalizeSchema(schema.items, root)
  const choice = variant === undefined ? null : objectVariants(items, root)
  const value =
    choice?.key !== undefined
      ? { [choice.key]: discriminator(choice.variants[variant!], choice.key, root) }
      : undefined
  return arrayItem(draftOf(schema.items, root, depth + 1, value, true))
}

/** The draft of a union field once another variant is chosen, keeping the values that fit. */
export function switchVariant(
  schema: JsonSchema,
  root: JsonSchema,
  depth: number,
  draft: Extract<Draft, { kind: "union" }>,
  variant: number | undefined
): Draft {
  const choice = objectVariants(schema, root)!
  if (variant === undefined) return { kind: "union", fields: {} }
  const fields = objectDrafts(choice.variants[variant], root, depth + 1, undefined, choice.key)
  for (const name of Object.keys(fields)) {
    const kept = draft.fields[name]
    if (kept && kept.kind === fields[name].kind) fields[name] = kept
  }
  return { kind: "union", variant, fields }
}

/**
 * Whether a field is as it started: blank, unchosen, or a required box left off. An optional
 * object of only such fields is not sent.
 */
export function emptyField(
  raw: unknown,
  root: JsonSchema,
  depth: number,
  draft: Draft | undefined,
  required: boolean
): boolean {
  if (!draft) return true
  const schema = normalizeSchema(raw, root)
  switch (draft.kind) {
    case "text":
      return draft.text.trim() === ""
    case "boolean":
      return required ? draft.value !== true : draft.value === undefined
    case "enum":
      return draft.index === undefined || (required && enumValues(schema)?.length === 1)
    case "object": {
      const needed = new Set(schema.required ?? [])
      return Object.entries(schema.properties ?? {}).every(([name, property]) =>
        emptyField(property, root, depth + 1, draft.fields[name], needed.has(name))
      )
    }
    case "union":
      return draft.variant === undefined
    case "array":
      return draft.items.length === 0
  }
}

/** A field that cannot be read, by its path such as `actions[1].x`. */
export type FieldError = { path: string; message: string }
type Read = { present: false } | { present: true; value: unknown }

/** A child's path below `path`. */
export const childPath = (path: string, name: string | number) =>
  typeof name === "number" ? `${path}[${name}]` : path ? `${path}.${name}` : name

function fail(errors: FieldError[], path: string, message: string): Read {
  errors.push({ path, message })
  return { present: false }
}

function readObject(
  schema: JsonSchema,
  root: JsonSchema,
  depth: number,
  fields: Record<string, Draft>,
  path: string,
  errors: FieldError[],
  skip?: string
): { value: Record<string, unknown>; failed: boolean } {
  const required = new Set(schema.required ?? [])
  const value: Record<string, unknown> = {}
  const before = errors.length
  for (const [name, property] of Object.entries(schema.properties ?? {})) {
    if (name === skip) continue
    const read = readDraft(
      property,
      root,
      depth,
      fields[name],
      required.has(name),
      childPath(path, name),
      errors
    )
    if (read.present) value[name] = read.value
  }
  return { value, failed: errors.length > before }
}

/**
 * The value a field sends, or nothing when it is left empty. A field that cannot be sent adds
 * its error. `bare` is an array item, which is sent even when empty.
 */
export function readDraft(
  raw: unknown,
  root: JsonSchema,
  depth: number,
  stored: Draft | undefined,
  required: boolean,
  path: string,
  errors: FieldError[],
  bare = false
): Read {
  const schema = normalizeSchema(raw, root)
  const kind = fieldKind(schema, root, depth)
  // A draft of another kind, such as after the server changed the schema, reads as empty.
  const draft =
    stored && stored.kind === draftKinds[kind]
      ? stored
      : draftOf(raw, root, depth, undefined, required)
  switch (kind) {
    case "string": {
      const text = (draft as Extract<Draft, { kind: "text" }>).text
      if (text === "")
        return required ? fail(errors, path, t("inspector.form.required")) : { present: false }
      return { present: true, value: text }
    }
    case "number": {
      const text = (draft as Extract<Draft, { kind: "text" }>).text.trim()
      if (text === "")
        return required ? fail(errors, path, t("inspector.form.required")) : { present: false }
      const value = Number(text)
      if (!Number.isFinite(value)) return fail(errors, path, t("inspector.form.number"))
      if (schema.type === "integer" && !Number.isInteger(value))
        return fail(errors, path, t("inspector.form.integer"))
      return { present: true, value }
    }
    case "boolean": {
      const value = (draft as Extract<Draft, { kind: "boolean" }>).value
      if (required) return { present: true, value: value === true }
      return value === undefined ? { present: false } : { present: true, value }
    }
    case "enum": {
      const index = (draft as Extract<Draft, { kind: "enum" }>).index
      if (index === undefined)
        return required ? fail(errors, path, t("inspector.form.required")) : { present: false }
      return { present: true, value: enumValues(schema)![index] }
    }
    case "object": {
      const fields = (draft as Extract<Draft, { kind: "object" }>).fields
      // An optional object left blank is not sent, even when it has required fields.
      if (!required && !bare && emptyField(raw, root, depth, draft, required))
        return { present: false }
      const { value, failed } = readObject(schema, root, depth + 1, fields, path, errors)
      if (failed) return { present: false }
      if (!Object.keys(value).length && !required && !bare) return { present: false }
      return { present: true, value }
    }
    case "union": {
      const union = draft as Extract<Draft, { kind: "union" }>
      const choice = objectVariants(schema, root)!
      const variant = union.variant === undefined ? undefined : choice.variants[union.variant]
      if (!variant)
        return required ? fail(errors, path, t("inspector.form.required")) : { present: false }
      const { value, failed } = readObject(
        variant,
        root,
        depth + 1,
        union.fields,
        path,
        errors,
        choice.key
      )
      if (failed) return { present: false }
      if (choice.key === undefined) return { present: true, value }
      return {
        present: true,
        value: { [choice.key]: discriminator(variant, choice.key, root), ...value },
      }
    }
    case "array": {
      const items = (draft as Extract<Draft, { kind: "array" }>).items
      if (!items.length) return required ? { present: true, value: [] } : { present: false }
      const before = errors.length
      const value = items.map((item, index) => {
        const read = readDraft(
          schema.items,
          root,
          depth + 1,
          item.draft,
          true,
          childPath(path, index),
          errors,
          true
        )
        return read.present ? read.value : null
      })
      return errors.length > before ? { present: false } : { present: true, value }
    }
    case "json": {
      const text = (draft as Extract<Draft, { kind: "text" }>).text
      if (text.trim() === "")
        return required ? fail(errors, path, t("inspector.form.required")) : { present: false }
      try {
        const value: unknown = JSON.parse(text)
        if (schema.type === "array" && !Array.isArray(value))
          return fail(errors, path, t("inspector.form.array"))
        if (schema.type === "object" && !isPlainObject(value))
          return fail(errors, path, t("inspector.form.object"))
        return { present: true, value }
      } catch (error) {
        // An untyped value or a union of plain ones that is not JSON goes as a string.
        if (!strictJson(schema)) return { present: true, value: text }
        return fail(
          errors,
          path,
          t("inspector.form.invalidJson", { error: (error as Error).message })
        )
      }
    }
  }
}

/** The form of a root object schema: its fields' drafts for `value`. */
export const formDraft = (schema: JsonSchema, value?: Record<string, unknown>) =>
  objectDrafts(schema, schema, 0, value ?? {})

/** The arguments a form sends, and the fields that stop it. */
export function readForm(schema: JsonSchema, fields: Record<string, Draft>) {
  const errors: FieldError[] = []
  const { value } = readObject(schema, schema, 0, fields, "", errors)
  return { value, errors }
}

/** Whether a root schema has fields the form can show. */
export const formFields = (schema: JsonSchema) =>
  Object.keys(normalizeSchema(schema, schema).properties ?? {}).length
