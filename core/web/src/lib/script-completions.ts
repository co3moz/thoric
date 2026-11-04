// What the script editors complete besides a script's own words: the commands Thoric puts in
// every desktop, the variables a script finds there, a compose step's own values and the lines
// that name a script's steps in the job panel. It is kept apart from Monaco so it can be tested;
// `monaco-setup.ts` hands these to Monaco as its completion items.
//
// `script-completions.json` names the commands and variables, and those left out on purpose, so
// the desktop image's tests and the manager's hold the list to what a desktop has.
import { PARAMETER_NAME, typeLabel, type ParameterDraft } from "./compose"
import data from "./script-completions.json"
import { t, type MessageKey } from "./i18n"
import type { ParameterType } from "./types"

/**
 * Where a script runs: a build, as root and without a screen; a compose's setup, in the desktop
 * once it has started; or a file of a running desktop, such as one a terminal runs.
 */
export type ScriptPlace = "build" | "setup" | "desktop"

/** A value a compose step's script reads from its environment. */
export type ScriptVariable = {
  name: string
  type: ParameterType
  label?: string
  description?: string
  secret?: boolean
  /** The volume parameter whose volume's name this variable holds. */
  volumeOf?: string
}

/** What an editor's script completes: where it runs and the compose values it reads. */
export type ScriptContext = { place: ScriptPlace; variables?: ScriptVariable[] }

/** A build's script, and a file of a running desktop. */
export const BUILD_SCRIPT: ScriptContext = { place: "build" }
export const DESKTOP_FILE: ScriptContext = { place: "desktop" }

export type CompletionKind =
  "parameter" | "variable" | "command" | "subcommand" | "snippet" | "word"

/**
 * A completion as the editor offers it. `start` and `end` are offsets in the cursor's line of the
 * text it replaces, `start` at or before the cursor and `end` at or after it.
 */
export type Completion = {
  label: string
  kind: CompletionKind
  insert: string
  /** `insert` is a snippet, with `${1:placeholder}` stops to fill in. */
  snippet?: boolean
  detail?: string
  /** Markdown. */
  documentation?: string
  filter?: string
  sort: string
  start: number
  end: number
}

type Command = {
  name: string
  places: ScriptPlace[]
  usage: string[]
  subcommands?: { name: string; key: string; insert: string }[]
}
type Variable = { name: string; places: ScriptPlace[] }

export const SCRIPT_COMMANDS = data.commands as Command[]
export const SCRIPT_VARIABLES = data.variables as Variable[]

/** The text of a command's or a variable's line, which every catalog has. */
export const commandKey = (name: string) => `completion.command.${name}` as MessageKey
export const commandDocKey = (name: string) => `completion.command.${name}.doc` as MessageKey
export const subcommandKey = (key: string) => `completion.subcommand.${key}` as MessageKey
export const variableKey = (name: string) => `completion.variable.${name}` as MessageKey

const NAME = new RegExp(`^${PARAMETER_NAME}$`)

/**
 * A compose step's parameters as the variables its script reads: each by its name, and a
 * volume's also by its name with `_VOLUME`, which holds the volume's.
 */
export function stepVariables(parameters: readonly ParameterDraft[]): ScriptVariable[] {
  const found: ScriptVariable[] = []
  for (const parameter of parameters) {
    if (!NAME.test(parameter.name)) continue
    const variable: ScriptVariable = { name: parameter.name, type: parameter.type }
    if (parameter.label.trim()) variable.label = parameter.label.trim()
    if (parameter.description.trim()) variable.description = parameter.description.trim()
    if (parameter.secret && parameter.type === "text") variable.secret = true
    found.push(variable)
    if (parameter.type === "volume")
      found.push({ name: `${parameter.name}_VOLUME`, type: "text", volumeOf: parameter.name })
  }
  return found.filter(
    (variable, index) => found.findIndex((other) => other.name === variable.name) === index
  )
}

/** Text as Markdown shows it literally, for a description someone typed. */
const literal = (text: string) => text.replace(/[\\`*_{}[\]<>()#+\-.!|~]/g, "\\$&")

/** Text as a snippet inserts it literally. */
const plain = (text: string) => text.replace(/[\\$}]/g, "\\$&")

const fence = (lines: readonly string[]) => ["```shell", ...lines, "```"].join("\n")

/** At most this many of a script's own words are offered, as Monaco limits its own. */
const WORD_LIMIT = 2000

/** The names a script gives its own variables: assigned, declared, looped over, read or used. */
export function scriptVariableNames(script: string): Set<string> {
  const found = new Set<string>()
  const add = (name: string | undefined) => name && found.add(name)
  // Comments are prose, whose `read the file` names no variable.
  const text = script.replace(/(^|\s)#.*$/gm, "$1")
  for (const match of text.matchAll(/(?:^|[\s;&|(])([A-Za-z_][A-Za-z0-9_]*)\+?=/gm)) add(match[1])
  for (const match of text.matchAll(
    /\b(?:export|local|declare|readonly|typeset)(?:[ \t]+-[A-Za-z]+)*((?:[ \t]+[A-Za-z_][A-Za-z0-9_]*)+)/g
  ))
    for (const name of match[1].trim().split(/\s+/)) add(name)
  for (const match of text.matchAll(/\b(?:for|select)[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]+in\b/g))
    add(match[1])
  for (const match of text.matchAll(/\bread\b([^\n;|&<>]*)/g))
    for (const part of match[1].trim().split(/\s+/))
      if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(part)) add(part)
  for (const match of text.matchAll(/\$\{?([A-Za-z_][A-Za-z0-9_]*)/g)) add(match[1])
  return found
}

/** A script's own words, as Monaco's word completion finds them, with dashed names kept whole. */
export function scriptWords(text: string): Set<string> {
  const found = new Set<string>()
  for (const [word] of text.matchAll(/[A-Za-z_](?:[A-Za-z0-9_-]*[A-Za-z0-9_])?/g)) {
    found.add(word)
    if (found.size >= WORD_LIMIT) break
  }
  return found
}

const offered = (places: readonly ScriptPlace[], context: ScriptContext) =>
  places.includes(context.place)

function parameterItem(variable: ScriptVariable) {
  if (variable.volumeOf) return { detail: t("completion.volumeName", { name: variable.volumeOf }) }
  const detail = t("completion.parameter", { type: typeLabel(variable.type) })
  const documentation = [
    variable.label && `**${literal(variable.label)}**`,
    variable.description && literal(variable.description),
    variable.secret && t("completion.parameterSecret"),
  ]
    .filter(Boolean)
    .join("\n\n")
  return { detail, documentation: documentation || undefined }
}

/**
 * Every variable a script of `context` reads, its compose values first, as completions of a name
 * at `start`; `dollar` puts the `$` before each, for a name typed without one.
 */
function variables(
  context: ScriptContext,
  own: Iterable<string>,
  skip: string,
  start: number,
  end: number,
  dollar: boolean
): Completion[] {
  const items: Completion[] = []
  const taken = new Set([skip])
  const add = (name: string, kind: CompletionKind, sort: string, more: Partial<Completion>) => {
    if (taken.has(name)) return
    taken.add(name)
    items.push({
      label: dollar ? `$${name}` : name,
      kind,
      insert: dollar ? `$${name}` : name,
      filter: name,
      sort: `${sort}${name}`,
      start,
      end,
      ...more,
    })
  }
  if (context.place === "setup")
    for (const variable of context.variables ?? [])
      add(variable.name, "parameter", "0", parameterItem(variable))
  for (const variable of SCRIPT_VARIABLES)
    if (offered(variable.places, context))
      add(variable.name, "variable", "1", { detail: t(variableKey(variable.name)) })
  // After a `$` the script's own variables follow; elsewhere they come with its other words.
  if (dollar) return items
  for (const name of own) add(name, "word", "3", {})
  return items
}

/** The commands Thoric puts in a desktop, as completions of the word at `start`. */
function commands(context: ScriptContext, start: number, end: number): Completion[] {
  return SCRIPT_COMMANDS.filter((command) => offered(command.places, context)).map((command) => ({
    label: command.name,
    kind: "command",
    insert: command.name,
    detail: t(commandKey(command.name)),
    documentation: `${t(commandDocKey(command.name))}\n\n${fence(command.usage)}`,
    sort: `1${command.name}`,
    start,
    end,
  }))
}

/** The lines that name a script's steps in the job panel, for a build's and a setup's script. */
function stepLines(before: string, after: string, start: number, cursor: number): Completion[] {
  // A line already begun with `echo` or `echo "` is completed whole, its closing quote too.
  const echo = /(?:^|[\s;&|])(echo[ \t]+("?))$/.exec(before.slice(0, start))
  const from = echo ? start - echo[1].length : start
  const end = cursor + (echo?.[2] && after.startsWith('"') ? 1 : 0)
  const lead = before.slice(from, start)
  const name = plain(t("completion.stepName"))
  return (["THORIC_STAGE", "THORIC_SKIP"] as const).map((marker) => {
    const step = marker === "THORIC_STAGE" ? 1 : 2
    return {
      label: marker,
      kind: "snippet" as const,
      insert: `echo "${marker}=\${1:${step}}/\${2:3} \${3:${name}}"`,
      snippet: true,
      detail: t(marker === "THORIC_STAGE" ? "completion.stage" : "completion.skip"),
      documentation: fence([`echo "${marker}=${step}/3 ${t("completion.stepName")}"`]),
      filter: `${lead}${marker}`,
      sort: `1${marker}`,
      start: from,
      end,
    }
  })
}

/**
 * The completions at the cursor of a script that runs as `context` says. `before` and `after`
 * are its line's text on either side of the cursor and `text` the whole script. After `$` or
 * `${` they are the names of variables; after `thoric-mcp`, its subcommands; anywhere else the
 * commands, the variables with their `$`, the step lines and the script's own words. A list
 * opened by typing a character, `triggered`, is offered only for a variable's name.
 */
export function scriptCompletions({
  before,
  after,
  text,
  context,
  triggered = false,
}: {
  before: string
  after: string
  text: string
  context: ScriptContext
  triggered?: boolean
}): Completion[] {
  const cursor = before.length
  const expansion = /\$\{?([A-Za-z_][A-Za-z0-9_]*)?$/.exec(before)
  if (expansion) {
    const typed = expansion[1] ?? ""
    const rest = /^[A-Za-z0-9_]*/.exec(after)![0]
    return variables(
      context,
      scriptVariableNames(text),
      typed + rest,
      cursor - typed.length,
      cursor,
      false
    )
  }
  if (triggered) return []
  const token = /[A-Za-z0-9_-]*$/.exec(before)![0]
  const start = cursor - token.length
  const whole = token + /^[A-Za-z0-9_-]*/.exec(after)![0]

  const mcp = SCRIPT_COMMANDS.find((command) => command.name === "thoric-mcp")
  if (
    mcp?.subcommands &&
    offered(mcp.places, context) &&
    /(?:^|[\s;&|(`])thoric-mcp[ \t]+$/.test(before.slice(0, start))
  )
    return mcp.subcommands.map((subcommand, index) => ({
      label: subcommand.name,
      kind: "subcommand",
      insert: subcommand.insert,
      snippet: true,
      detail: t(subcommandKey(subcommand.key)),
      filter: subcommand.name,
      sort: `0${index}`,
      start,
      end: cursor,
    }))

  const items = [
    ...variables(context, [], whole, start, cursor, true),
    ...commands(context, start, cursor),
    ...(context.place === "desktop" ? [] : stepLines(before, after, start, cursor)),
  ]
  const taken = new Set([whole, ...items.flatMap((item) => [item.label, item.filter ?? ""])])
  for (const word of scriptWords(text))
    if (!taken.has(word))
      items.push({ label: word, kind: "word", insert: word, sort: `3${word}`, start, end: cursor })
  return items
}
