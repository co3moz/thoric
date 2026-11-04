import { test } from "bun:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"

import { newParameter } from "../src/lib/compose"
import { has } from "../src/lib/i18n"
import data from "../src/lib/script-completions.json"
import {
  BUILD_SCRIPT,
  commandDocKey,
  commandKey,
  DESKTOP_FILE,
  SCRIPT_COMMANDS,
  SCRIPT_VARIABLES,
  scriptCompletions,
  scriptVariableNames,
  scriptWords,
  stepVariables,
  subcommandKey,
  variableKey,
  type ScriptContext,
} from "../src/lib/script-completions"

const ROOT = new URL("../../../", import.meta.url)
const read = (path: string) => readFileSync(new URL(path, ROOT), "utf8")

const SETUP: ScriptContext = {
  place: "setup",
  variables: [
    { name: "UNITY_EMAIL", type: "text", label: "Unity account" },
    { name: "UNITY_PASSWORD", type: "text", secret: true },
  ],
}

/** The completions with the cursor at `|` in `line`, in a script of `text`. */
function at(line: string, context: ScriptContext, text = line.replace("|", ""), triggered = false) {
  const cursor = line.indexOf("|")
  return scriptCompletions({
    before: line.slice(0, cursor),
    after: line.slice(cursor + 1),
    text,
    context,
    triggered,
  })
}
const labels = (items: { label: string }[]) => items.map((item) => item.label)

test("after a $ the step's own values come first, then the desktop's and the script's own", () => {
  const script =
    'set -eu\neditor=$(ls -d ~/Unity | head -n 1)\nfor item in a b; do :; done\necho "$'
  const items = at('echo "$|', SETUP, script, true)
  const names = labels(items)
  assert.deepEqual(names.slice(0, 2), ["UNITY_EMAIL", "UNITY_PASSWORD"])
  assert.ok(names.includes("DESKTOP_GPU"))
  assert.ok(names.includes("editor") && names.includes("item"))
  // Names only: no commands, no `$`, and the text replaced starts after the `$`.
  assert.ok(!names.includes("gpu-run"))
  assert.ok(items.every((item) => !item.insert.startsWith("$") && item.start === 7))
  assert.equal(items[0].kind, "parameter")
  assert.match(items[1].documentation ?? "", /masks/)
})

test("a name inside ${ is completed where it is, before the closing brace", () => {
  const items = at("x=${DESK|}", SETUP)
  const gpu = items.find((item) => item.label === "DESKTOP_GPU")
  assert.equal(gpu?.insert, "DESKTOP_GPU")
  assert.equal(gpu?.start, 4)
  assert.equal(gpu?.end, 8)
})

test("a build completes what it has without a screen, a setup the whole desktop", () => {
  const build = labels(at("|", BUILD_SCRIPT, ""))
  assert.ok(build.includes("gpu-run"))
  assert.ok(build.includes("$DESKTOP_SESSION"))
  assert.ok(build.includes("THORIC_STAGE") && build.includes("THORIC_SKIP"))
  for (const missing of ["thoric-mcp", "desktop-screenshot", "$DISPLAY", "$UNITY_EMAIL"])
    assert.ok(!build.includes(missing), missing)

  const setup = labels(at("|", SETUP, ""))
  for (const name of ["thoric-mcp", "desktop-screenshot", "$DISPLAY", "$UNITY_EMAIL"])
    assert.ok(setup.includes(name), name)
  // A file of a desktop has its commands, but no compose values or step lines.
  const file = labels(at("|", DESKTOP_FILE, ""))
  assert.ok(file.includes("thoric-mcp"))
  assert.ok(!file.includes("THORIC_STAGE"))
})

test("a dashed command is completed whole, with its usage", () => {
  const items = at("gpu-r|", BUILD_SCRIPT)
  const command = items.find((item) => item.label === "gpu-run")
  assert.equal(command?.start, 0)
  assert.match(command?.documentation ?? "", /```shell\ngpu-run glxinfo -B/)
})

test("thoric-mcp without its subcommand offers them, each a snippet to fill in", () => {
  const items = at("thoric-mcp |", DESKTOP_FILE)
  assert.deepEqual(labels(items), ["add", "add --url", "remove", "list", "show"])
  assert.ok(items.every((item) => item.snippet && item.kind === "subcommand"))
  assert.equal(items[1].insert, "add ${1:NAME} --url http://127.0.0.1:${2:PORT}/mcp")
  // Not where the command is not offered, and not once a subcommand is there.
  assert.ok(!labels(at("thoric-mcp |", BUILD_SCRIPT)).includes("add"))
  assert.ok(!labels(at("thoric-mcp add |", DESKTOP_FILE)).includes("remove"))
})

test("a step line begun with echo is completed whole, its closing quote too", () => {
  const items = at('  echo "THORIC_S|"', BUILD_SCRIPT)
  const stage = items.find((item) => item.label === "THORIC_STAGE")
  assert.equal(stage?.start, 2)
  assert.equal(stage?.end, 17)
  assert.equal(stage?.filter, 'echo "THORIC_STAGE')
  assert.equal(stage?.insert, 'echo "THORIC_STAGE=${1:1}/${2:3} ${3:Step name}"')
  // Without the echo, the snippet writes it.
  assert.equal(at("THORIC_S|", BUILD_SCRIPT).find((item) => item.label === "THORIC_SKIP")?.start, 0)
})

test("a typed character that opens no variable name offers nothing", () => {
  assert.deepEqual(at("function f() {|", SETUP, undefined, true), [])
  assert.ok(at("${|", SETUP, undefined, true).length > 0)
})

test("the script's own words follow, dashed ones whole, without the word being typed", () => {
  const script = "apt-get install -y ca-certificates\nadd_panel_launcher godot\ngpu-run godot\nad"
  const items = at("ad|", BUILD_SCRIPT, script)
  const words = items.filter((item) => item.kind === "word").map((item) => item.label)
  assert.ok(words.includes("apt-get") && words.includes("ca-certificates"))
  assert.ok(words.includes("add_panel_launcher"))
  // The word at the cursor and the commands already offered are not repeated.
  assert.ok(!words.includes("ad") && !words.includes("gpu-run"))
  // They sort after everything Thoric offers.
  const last = Math.max(
    ...items.filter((item) => item.kind !== "word").map((item) => item.sort.charCodeAt(0))
  )
  assert.ok(items.every((item) => item.kind !== "word" || item.sort.charCodeAt(0) > last))
  assert.deepEqual([...scriptWords("x 42 y-z -n")], ["x", "y-z", "n"])
})

test("a script's own variables are those it assigns, declares, loops over, reads or uses", () => {
  const names = scriptVariableNames(
    [
      "# read the docs first",
      "export DEBIAN_FRONTEND=noninteractive",
      "local -r one two",
      "for item in a b; do :; done",
      'IFS=, read -r x y <<<"$region"',
      'echo "${HOME}"',
    ].join("\n")
  )
  assert.deepEqual(
    [...names].sort(),
    ["DEBIAN_FRONTEND", "HOME", "IFS", "item", "one", "region", "two", "x", "y"].sort()
  )
})

test("a step's parameters are its variables, a volume's with the variable of its name", () => {
  const parameter = (patch: Partial<ReturnType<typeof newParameter>>) => ({
    ...newParameter(),
    ...patch,
  })
  assert.deepEqual(
    stepVariables([
      parameter({ name: "UNITY_HOME", type: "volume", label: " Unity installs " }),
      parameter({ name: "TOKEN", secret: true, description: "From the *account* page" }),
      parameter({ name: "lower" }),
      parameter({ name: "" }),
      parameter({ name: "TOKEN" }),
    ]),
    [
      { name: "UNITY_HOME", type: "volume", label: "Unity installs" },
      { name: "UNITY_HOME_VOLUME", type: "text", volumeOf: "UNITY_HOME" },
      { name: "TOKEN", type: "text", description: "From the *account* page", secret: true },
    ]
  )
  // Someone's description is shown as they wrote it, not as Markdown.
  const items = at("$|", {
    place: "setup",
    variables: [{ name: "TOKEN", type: "text", description: "From the *account* page" }],
  })
  assert.match(items[0].documentation ?? "", /From the \\\*account\\\* page/)
})

test("every command and variable the editor offers has its text in the catalog", () => {
  for (const command of SCRIPT_COMMANDS) {
    assert.ok(has(commandKey(command.name)), command.name)
    assert.ok(has(commandDocKey(command.name)), command.name)
    for (const subcommand of command.subcommands ?? [])
      assert.ok(has(subcommandKey(subcommand.key)), subcommand.key)
  }
  for (const variable of SCRIPT_VARIABLES) assert.ok(has(variableKey(variable.name)), variable.name)
})

test("every command the editor offers or leaves out is one the desktop image installs", () => {
  const dockerfile = read("images/desktop/Dockerfile")
  const installed = [...dockerfile.matchAll(/^COPY \S+ \/usr\/local\/bin\/(\S+)$/gm)].map(
    (match) => match[1]
  )
  const named = [...SCRIPT_COMMANDS.map((command) => command.name), ...data.unlisted.commands]
  assert.deepEqual([...named].sort(), [...installed].sort())
  // Its subcommands are those thoric-mcp has.
  const parsers = [...read("images/desktop/thoric-mcp.py").matchAll(/add_parser\("(\w+)"\)/g)]
  const mcp = SCRIPT_COMMANDS.find((command) => command.name === "thoric-mcp")!
  assert.deepEqual(
    [...new Set(mcp.subcommands!.map((subcommand) => subcommand.name.split(" ")[0]))].sort(),
    parsers.map((match) => match[1]).sort()
  )
})

test("every variable the editor offers is one a desktop is given", () => {
  const sources = [
    read("images/desktop/Dockerfile"),
    read("core/manager/src/docker/create_args.rs"),
    read("core/manager/src/outbound.rs"),
  ].join("\n")
  for (const variable of SCRIPT_VARIABLES)
    assert.match(sources, new RegExp(`\\b${variable.name}\\b`), variable.name)
})
