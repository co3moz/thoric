import { test } from "bun:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"

import { en } from "../src/locales/en"
import {
  changesOf,
  choiceIndex,
  featureDraft,
  gpuNotes,
  FIELDS,
  idleSettings,
  initialText,
  NOTIFY_EVENTS,
  notifyEvents,
  releaseState,
  SECTION_OF,
  sections,
  settingRows,
  shownValue,
  textProblem,
} from "../src/lib/settings"
import type { ConfigVariable, GpuReport, Settings } from "../src/lib/types"

test("every setting the page knows is still an installation setting and says what it does", () => {
  const example = readFileSync(new URL("../../../.env.example", import.meta.url), "utf8")
  for (const name of Object.keys(SECTION_OF)) {
    assert.ok(example.includes(`\n${name}=`), `.env.example no longer has ${name}`)
    assert.ok(`settings.about.${name}` in en, `settings.about.${name} is missing`)
  }
})

test("settings are listed by section in the manager's order, unknown ones last", () => {
  const variables: ConfigVariable[] = [
    { name: "UPDATE_CHECK", value: true },
    { name: "AUTH_MODE", value: "password" },
    { name: "ADMIN_PASSWORD", secret: true, set: true },
    { name: "SOMETHING_NEWER", value: "x" },
    { name: "ROOT_MCP_ENABLED", value: false },
  ]
  assert.deepEqual(
    sections(variables).map(({ section, variables }) => [
      section,
      variables.map((variable) => variable.name),
    ]),
    [
      ["access", ["AUTH_MODE", "ADMIN_PASSWORD"]],
      ["mcp", ["ROOT_MCP_ENABLED"]],
      ["manager", ["UPDATE_CHECK"]],
      ["misc", ["SOMETHING_NEWER"]],
    ]
  )
})

test("a secret shows only whether it is set, and empty values read as not set", () => {
  assert.deepEqual(shownValue({ name: "MCP_TOKEN", secret: true, set: false }), {
    kind: "secret",
    set: false,
  })
  assert.deepEqual(shownValue({ name: "GPU_ADAPTER", value: "" }), { kind: "empty" })
  assert.deepEqual(shownValue({ name: "MANAGER_CONTAINER", value: null }), { kind: "empty" })
  assert.deepEqual(shownValue({ name: "WSL_GPU", value: false }), { kind: "text", text: "false" })
  assert.deepEqual(shownValue({ name: "UPLOAD_MAX_MB", value: 1024 }), {
    kind: "text",
    text: "1024",
  })
})

test("the release state follows the checks, a running update and the last answer", () => {
  const status = { enabled: true, job_id: null, available: false, newest: null, error: null }
  assert.equal(releaseState({ ...status, enabled: false, available: true }), "off")
  assert.equal(releaseState(status), "unchecked")
  assert.equal(releaseState({ ...status, error: "GitHub is unreachable" }), "failed")
  assert.equal(releaseState({ ...status, newest: "0.6.2" }), "current")
  // A failed look keeps the release seen before.
  assert.equal(releaseState({ ...status, newest: "0.6.2", error: "rate limited" }), "current")
  assert.equal(releaseState({ ...status, newest: "0.7.0", available: true }), "available")
  assert.equal(
    releaseState({ ...status, newest: "0.7.0", available: true, job_id: "j" }),
    "updating"
  )
})

test("every control the page has is for a setting the manager lets it change", () => {
  // EDITABLE in core/manager/src/config.rs is what the manager accepts; a control for anything
  // else would only ever be refused.
  const config = readFileSync(new URL("../../manager/src/config.rs", import.meta.url), "utf8")
  const block = /^pub\(crate\) const EDITABLE: &\[&str\] = &\[\n([\s\S]*?)^\];$/m.exec(config)
  assert.ok(block, "EDITABLE in core/manager/src/config.rs")
  const editable = [...block[1].matchAll(/^ {4}"([A-Z_]+)",$/gm)].map((match) => match[1])
  for (const name of Object.keys(FIELDS)) assert.ok(editable.includes(name), name)
  for (const field of Object.values(FIELDS))
    if (field.kind === "choice")
      for (const choice of field.choices) if (choice.label) assert.ok(choice.label in en)
})

test("a setting starts from the text its environment gave, or what the manager made of it", () => {
  const variable = (name: string, value: string | number | boolean, text: string) =>
    ({ name, value, editable: true, text }) as ConfigVariable
  assert.equal(initialText(variable("TEMPLATES_URL", "https://wiki/index.json", "")), "")
  assert.equal(initialText(variable("UPLOAD_MAX_MB", 1024, "")), "1024")
  assert.equal(initialText(variable("SECURE_COOKIE", true, "")), "true")
  assert.equal(initialText(variable("SECURE_COOKIE", true, "1")), "1")
  assert.equal(initialText(variable("DESKTOP_IMAGE", "ghcr.io/x/y:1", "")), "ghcr.io/x/y:1")
  assert.equal(initialText(variable("GPU_ADAPTER", "", "")), "")
  assert.equal(initialText({ name: "MCP_TOKEN", secret: true, set: true }), "")
})

test("a draft changes only what differs, and only editable settings", () => {
  const variables: ConfigVariable[] = [
    { name: "SECURE_COOKIE", value: false, editable: true, text: "false" },
    { name: "UPLOAD_MAX_MB", value: 1024, editable: true, text: "1024" },
    { name: "AUTH_MODE", value: "password", editable: false },
    { name: "ADMIN_PASSWORD", secret: true, set: true },
    { name: "NOTIFY_URL", secret: true, set: true, editable: true },
  ]
  assert.deepEqual(
    changesOf(variables, {
      SECURE_COOKIE: "true",
      UPLOAD_MAX_MB: "1024",
      AUTH_MODE: "none",
      ADMIN_PASSWORD: "x",
    }),
    { SECURE_COOKIE: "true" }
  )
  // The page never has an editable secret's value, so one in the draft replaces it, and an empty
  // one removes it.
  assert.deepEqual(changesOf(variables, { NOTIFY_URL: "https://ntfy.sh/phone" }), {
    NOTIFY_URL: "https://ntfy.sh/phone",
  })
  assert.deepEqual(changesOf(variables, { NOTIFY_URL: "" }), { NOTIFY_URL: "" })
  assert.deepEqual(changesOf(variables, {}), {})
})

test("a text is refused before the manager is asked, as .env would misread it", () => {
  for (const text of ["", "true", "GeForce RTX", "https://example.com/index.json", "team=a"])
    assert.equal(textProblem("GPU_ADAPTER", text), null, text)
  for (const text of [
    "$HOME",
    "a#b",
    "'a'",
    '"a"',
    "a`b`",
    String.raw`a\b`,
    " a",
    "a ",
    "a\nb",
    "café",
  ])
    assert.equal(textProblem("GPU_ADAPTER", text)?.key, "settings.edit.problem.text", text)
  assert.equal(textProblem("UPLOAD_MAX_MB", "2048"), null)
  for (const text of ["0", "65537", "1.5", "", "lots"])
    assert.deepEqual(textProblem("UPLOAD_MAX_MB", text), {
      key: "settings.edit.problem.number",
      values: { min: 1, max: 65536 },
    })
})

test("a choice reads its own values and leaves the rest to the user's text", () => {
  const field = FIELDS.TEMPLATES_URL
  assert.ok(field.kind === "choice")
  assert.equal(choiceIndex(field, ""), 0)
  assert.equal(choiceIndex(field, "none"), 1)
  assert.equal(choiceIndex(field, "https://example.com/index.json"), -1)
})

test("a new container starts from the installation's defaults, and a manager without them as before", () => {
  const settings = {
    gpu_enabled: true,
    defaults: {
      gpu: false,
      internet: false,
      sudo: true,
      audio: true,
      microphone: true,
      gamepad: true,
      vpn: true,
    },
  } as Settings
  assert.deepEqual(featureDraft({}, settings), {
    gpu: false,
    internet: false,
    sudo: true,
    audio: true,
    microphone: true,
    gamepad: true,
    vpn: true,
  })
  // A container's own settings win, and the microphone stays off without sound.
  assert.deepEqual(featureDraft({ audio: false, internet: true }, settings), {
    gpu: false,
    internet: true,
    sudo: true,
    audio: false,
    microphone: false,
    gamepad: true,
    vpn: true,
  })
  assert.deepEqual(featureDraft({}, { gpu_enabled: false } as Settings), {
    gpu: false,
    internet: true,
    sudo: true,
    audio: false,
    microphone: false,
    gamepad: false,
    vpn: false,
  })
})

test("the GPU card names every missing route only when desktops get no GPU", () => {
  const unused: GpuReport["unused"] = [
    { route: "wsl", setting: "WSL_GPU", reason: "no_wsl_gpu" },
    { route: "dri", setting: "DRI_GPU", reason: "no_render_device" },
  ]
  // A Linux host without GPUs says why each setting found nothing.
  assert.deepEqual(gpuNotes({ routes: [], unused, found: null }), { routes: [], unused })
  // WSL without render devices is what the defaults expect, and goes unsaid.
  assert.deepEqual(gpuNotes({ routes: ["nvidia", "wsl"], unused: unused.slice(1), found: null }), {
    routes: ["nvidia", "wsl"],
    unused: [],
  })
  // An NVIDIA GPU Docker cannot pass is worth a word beside a route that works.
  const toolkit = { route: "nvidia", setting: "NVIDIA_COMPUTE", reason: "no_toolkit" } as const
  assert.deepEqual(gpuNotes({ routes: ["dri"], unused: [toolkit], found: null }).unused, [toolkit])
  for (const reason of [
    "no_wsl_gpu",
    "no_nvidia_gpu",
    "no_toolkit",
    "no_render_device",
    "beside_wsl",
    "unseen",
  ])
    assert.ok(`settings.gpu.reason.${reason}` in en, reason)
  for (const route of ["wsl", "nvidia", "dri"])
    assert.ok(`settings.gpu.route.${route}` in en, route)
})

/** The container features as the manager lists them, every switch on. */
const FEATURE_SWITCHES: ConfigVariable[] = [
  "AUDIO_ENABLED",
  "MICROPHONE_ENABLED",
  "INTERNET_ENABLED",
  "SUDO_ENABLED",
  "GAMEPAD_ENABLED",
  "VPN_ENABLED",
  "GPU_DEFAULT",
  "INTERNET_DEFAULT",
  "SUDO_DEFAULT",
  "AUDIO_DEFAULT",
  "MICROPHONE_DEFAULT",
  "GAMEPAD_DEFAULT",
  "VPN_DEFAULT",
].map((name) => ({ name, value: true, editable: true }))

test("a feature's switch and its default for new containers share a row", () => {
  const names = (variables: ConfigVariable[]) =>
    settingRows(variables).map((row) => row.map((variable) => variable.name))
  assert.deepEqual(names(FEATURE_SWITCHES), [
    ["AUDIO_ENABLED", "AUDIO_DEFAULT"],
    ["MICROPHONE_ENABLED", "MICROPHONE_DEFAULT"],
    ["INTERNET_ENABLED", "INTERNET_DEFAULT"],
    ["SUDO_ENABLED", "SUDO_DEFAULT"],
    ["GAMEPAD_ENABLED", "GAMEPAD_DEFAULT"],
    ["VPN_ENABLED", "VPN_DEFAULT"],
    // The GPU has routes rather than a switch of its own.
    ["GPU_DEFAULT"],
  ])
  // A pair stands where the first of the two was; a switch without a default stands alone.
  assert.deepEqual(
    names([
      { name: "ROOT_MCP_ENABLED", value: false },
      { name: "WEBCAM_DEFAULT", value: true },
      { name: "UPLOAD_MAX_MB", value: 1024 },
      { name: "WEBCAM_ENABLED", value: true },
    ]),
    [["ROOT_MCP_ENABLED"], ["WEBCAM_ENABLED", "WEBCAM_DEFAULT"], ["UPLOAD_MAX_MB"]]
  )
})

test("a default goes idle with its feature, and the microphone with sound", () => {
  const idle = (...off: string[]) => idleSettings(FEATURE_SWITCHES, (name) => !off.includes(name))
  assert.deepEqual(idle(), {})
  assert.deepEqual(idle("INTERNET_ENABLED", "GAMEPAD_ENABLED"), {
    INTERNET_DEFAULT: "INTERNET_ENABLED",
    GAMEPAD_DEFAULT: "GAMEPAD_ENABLED",
  })
  assert.deepEqual(idle("MICROPHONE_ENABLED"), { MICROPHONE_DEFAULT: "MICROPHONE_ENABLED" })
  // Without sound the microphone is off too, whatever its own switch says.
  assert.deepEqual(idle("AUDIO_ENABLED"), {
    MICROPHONE_ENABLED: "AUDIO_ENABLED",
    AUDIO_DEFAULT: "AUDIO_ENABLED",
    MICROPHONE_DEFAULT: "AUDIO_ENABLED",
  })
  // A default whose switch is not listed, such as the GPU's, never goes idle.
  assert.deepEqual(
    idleSettings([{ name: "GPU_DEFAULT", value: true }], () => false),
    {}
  )
})

test("the notices sent are named by their labels, and a kind this page does not know by itself", () => {
  assert.equal(notifyEvents(""), null)
  assert.equal(notifyEvents("none"), null)
  assert.deepEqual(notifyEvents("human_request, container_lifecycle, build_finished"), [
    { name: "human_request", label: "notify.event.human_request" },
    { name: "container_lifecycle", label: "notify.event.container_lifecycle" },
    { name: "build_finished" },
  ])
  // Every kind with a label is one the manager still sends (EVENTS in core/manager/src/notify.rs).
  const notify = readFileSync(new URL("../../manager/src/notify.rs", import.meta.url), "utf8")
  for (const [name, label] of Object.entries(NOTIFY_EVENTS)) {
    assert.ok(notify.includes(`, "${name}", `), `${name} is no longer a kind of notice`)
    assert.ok(label in en, label)
  }
})
