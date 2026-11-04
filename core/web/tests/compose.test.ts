import { test } from "bun:test"
import assert from "node:assert/strict"

import {
  browserPattern,
  displayOf,
  draftParameter,
  NEW_VOLUME,
  newOption,
  newVolumeName,
  NO_VOLUME,
  noVolumeValue,
  parameterBody,
  patternMatches,
  readCompose,
  sliderValue,
  composeBody,
  composeDraft,
  composeEntries,
  composeFor,
  composeImage,
  composeSteps,
  inheritedSteps,
  missingValues,
  setupVolumes,
  valueMissing,
  imageSteps,
  orderSteps,
  setupGroups,
  toggleChoice,
  volumeChoices,
  volumeDefault,
  volumeExtras,
  volumeNameValid,
  volumeSelection,
  withOptions,
  withType,
  withVolumeDefault,
  WITHOUT_VOLUME,
} from "../src/lib/compose"
import type { Compose, ComposeParameter } from "../src/lib/types"

const unity: Compose = {
  parameters: [
    {
      name: "UNITY_EMAIL",
      label: "Unity email",
      description: "The Unity ID you sign in with.",
      type: "text",
      required: true,
      secret: false,
      default: "",
    },
    {
      name: "UNITY_PASSWORD",
      label: "",
      description: "",
      type: "text",
      required: true,
      secret: true,
      default: "",
    },
    {
      name: "BRANCH",
      label: "Branch",
      description: "",
      type: "text",
      required: false,
      secret: false,
      default: "main",
    },
  ],
  script: "set -eu\n",
  root: false,
  timeout_seconds: 600,
}

const record = (id: string, compose?: Compose) => ({
  image_id: `sha256:${id}`,
  reference: `thoric/snapshot:${id}`,
  compose: compose ?? null,
})

test("the compose comes from a record of the chosen image, as the manager picks it", () => {
  const records = [record("plain"), record("unity", unity), record("again", unity)]
  assert.equal(composeFor(records, "sha256:unity"), unity)
  assert.equal(composeFor(records, "thoric/snapshot:again"), unity)
  // A catalog reference or an image without one asks for nothing.
  assert.equal(composeFor(records, "sha256:plain"), undefined)
  assert.equal(composeFor(records, "ghcr.io/co3moz/thoric/ubuntu-xfce:24.04"), undefined)
  assert.equal(composeFor(records, undefined), undefined)
  // Two records of one image: the first, newest one with a compose wins.
  const shared = [
    { ...record("same"), reference: "a" },
    { ...record("same", { ...unity, root: true }), reference: "b" },
  ]
  assert.equal(composeSteps(composeFor(shared, "sha256:same")!)[0].root, true)
})

test("a build keeps the compose of the record a container would take it from, by its steps", () => {
  const re = { ...record("re", unity), name: "re" }
  const records = [record("plain"), re]
  assert.equal(composeImage(records, "sha256:re"), re)
  assert.equal(composeImage(records, "sha256:plain"), undefined)
  // The one step of a compose that names none goes by the image's name, as the manager names it.
  assert.deepEqual(imageSteps(re), ["re"])
  const merged = {
    name: "merged",
    compose: {
      steps: [
        { ...unity, id: "unity-dev", name: "Unity development" },
        { ...unity, id: "chrome", name: "" },
      ],
    },
  }
  assert.deepEqual(imageSteps(merged), ["Unity development", "chrome"])
  assert.deepEqual(imageSteps({ name: "plain", compose: null }), [])
})

test("the create form sends a value for every parameter, the default when left alone", () => {
  assert.deepEqual(parameterBody(unity, {}), {
    UNITY_EMAIL: "",
    UNITY_PASSWORD: "",
    BRANCH: "main",
  })
  assert.deepEqual(parameterBody(unity, { UNITY_EMAIL: "dev@example.com", BRANCH: "" }), {
    UNITY_EMAIL: "dev@example.com",
    UNITY_PASSWORD: "",
    BRANCH: "",
  })
  // Values left from an image chosen earlier are not sent.
  assert.deepEqual(Object.keys(parameterBody(unity, { OTHER: "x" })), [
    "UNITY_EMAIL",
    "UNITY_PASSWORD",
    "BRANCH",
  ])
  assert.deepEqual(parameterBody(undefined, { OTHER: "x" }), {})
})

test("a compose survives the editor unchanged, and a secret loses its default", () => {
  assert.deepEqual(composeBody(composeDraft(unity)), unity)
  const draft = composeDraft(unity)
  draft.steps[0].parameters[2] = { ...draft.steps[0].parameters[2], secret: true, name: " TOKEN " }
  const body = composeBody(draft)
  assert.deepEqual(composeSteps(body)[0].parameters[2], {
    name: "TOKEN",
    label: "Branch",
    description: "",
    type: "text",
    required: false,
    secret: true,
    default: "",
  })
  // A new compose starts with a script skeleton and the manager's defaults.
  const fresh = composeDraft()
  assert.deepEqual(
    [fresh.steps[0].parameters, fresh.steps[0].root, fresh.steps[0].timeout_seconds],
    [[], false, "600"]
  )
  assert.match(fresh.steps[0].script, /set -euo pipefail/)
})

test("a pasted compose fills in what it leaves out and says what is wrong", () => {
  assert.deepEqual(readCompose({ script: "true", parameters: [{ name: "A" }] }), {
    parameters: [
      {
        name: "A",
        label: "",
        description: "",
        type: "text",
        required: false,
        secret: false,
        default: "",
      },
    ],
    script: "true",
    root: false,
    timeout_seconds: 600,
  })
  const cases: [unknown, RegExp][] = [
    [null, /JSON object/],
    [{ parameters: [] }, /no setup script/],
    [{ script: "true", parameters: {} }, /must be a list/],
    [{ script: "true", parameters: [{ label: "x" }] }, /needs a name/],
    [{ script: "true", parameters: [{ name: "A", secret: "yes" }] }, /parameter A/],
    [{ script: "true", root: "no" }, /true or false/],
    [{ script: "true", timeout_seconds: "600" }, /number of seconds/],
  ]
  for (const [value, message] of cases)
    assert.throws(() => readCompose(value), message, JSON.stringify(value))
})

const flavors = [
  { value: "vanilla", label: "Vanilla" },
  { value: "chocolate", label: "" },
]

// Each type with the fields only it has, as the manager writes them back.
const typed: ComposeParameter[] = [
  {
    name: "NOTES",
    label: "",
    description: "",
    type: "text",
    display: "multiline",
    required: false,
    secret: false,
    default: "a\nb",
  },
  {
    name: "THREADS",
    label: "",
    description: "",
    type: "number",
    display: "slider",
    required: true,
    secret: false,
    default: "4",
    min: 1,
    max: 16,
    step: 1,
  },
  {
    name: "PORT",
    label: "",
    description: "",
    type: "number",
    required: false,
    secret: false,
    default: "",
    min: 1024,
  },
  {
    name: "ACCEPT",
    label: "",
    description: "",
    type: "boolean",
    display: "checkbox",
    required: true,
    secret: false,
    default: "",
  },
  {
    name: "FLAVOR",
    label: "",
    description: "",
    type: "choice",
    display: "radio",
    required: false,
    secret: false,
    default: "mint",
    options: flavors,
    allow_custom: true,
  },
  {
    name: "TOPPINGS",
    label: "",
    description: "",
    type: "choices",
    required: false,
    secret: false,
    default: "chocolate",
    options: flavors,
  },
  {
    name: "ACCELERATOR",
    label: "",
    description: "",
    type: "text",
    required: false,
    secret: false,
    default: "192.168.1.20:10080",
    pattern: "[^:]+:[0-9]+",
    pattern_message: "host:port, for example 192.168.1.20:10080",
  },
]

const installs: ComposeParameter = {
  name: "UNITY_HOME",
  label: "Unity installs",
  description: "",
  type: "volume",
  required: false,
  secret: false,
  default: "unity-installs",
  path: "/opt/unity",
}

test("a volume parameter keeps its path through the editor and a pasted configuration", () => {
  const compose = { ...unity, parameters: [...typed, installs] }
  assert.deepEqual(composeBody(composeDraft(compose)), compose)
  assert.deepEqual(
    composeSteps(readCompose({ script: "true", parameters: [installs] }))[0].parameters,
    [installs]
  )
  // Only a volume parameter sends a path, though the draft keeps it while the type changes.
  const [draft] = composeDraft({ ...unity, parameters: [installs] }).steps[0].parameters
  const text = withType(draft, "text")
  assert.equal("path" in draftParameter(text), false)
  assert.equal(draftParameter(withType(text, "volume")).path, "/opt/unity")
  assert.equal(displayOf({ type: "volume" }), "select")
  assert.throws(
    () => readCompose({ script: "true", parameters: [{ ...installs, path: 5 }] }),
    /parameter UNITY_HOME/
  )
})

test("a folder parameter is plain text the editor and a pasted configuration keep", () => {
  const project: ComposeParameter = {
    name: "UNITY_PROJECT",
    label: "Unity project",
    description: "",
    type: "folder",
    required: true,
    secret: false,
    default: "/home/desktop/UnityProjects/MyGame",
  }
  const compose = { ...unity, parameters: [installs, project] }
  assert.deepEqual(composeBody(composeDraft(compose)), compose)
  assert.deepEqual(
    composeSteps(readCompose({ script: "true", parameters: [project] }))[0].parameters,
    [project]
  )
  assert.equal(displayOf({ type: "folder" }), "input")
  // A secret text parameter turned into a folder is no longer secret, and its default goes.
  const [draft] = composeDraft({ ...unity, parameters: [typed[0]] }).steps[0].parameters
  const folder = draftParameter(withType({ ...draft, secret: true }, "folder"))
  assert.equal(folder.secret, false)
  assert.equal(folder.default, "")
  assert.equal("pattern" in folder, false)
})

test("a volume parameter offers the volumes there are and names a new one after itself", () => {
  const volumes = [{ Name: "projects" }, { Name: "Assets" }, { Name: "unity-home" }]
  assert.deepEqual(volumeChoices(volumes, ""), ["Assets", "projects", "unity-home"])
  // A value whose volume is gone stays visible, so the form does not silently drop it.
  assert.deepEqual(volumeChoices([], "unity-installs"), ["unity-installs"])
  assert.equal(newVolumeName({ name: "UNITY_INSTALLS" }, volumes), "unity-installs")
  assert.equal(newVolumeName({ name: "UNITY_HOME" }, volumes), "unity-home-2")
  assert.equal(newVolumeName({ name: "X" }, []), "x-volume")
  for (const name of ["unity-installs", "x-volume", "unity-home-2"])
    assert.ok(volumeNameValid(name), name)
  for (const name of ["", "a", "-unity", "with space", "a/b", "x".repeat(129)])
    assert.equal(volumeNameValid(name), false, name)
})

const workspace: ComposeParameter = {
  name: "UNITY_WORKSPACE",
  label: "Workspace",
  description: "",
  type: "volume",
  required: true,
  secret: false,
  default: "",
  path: "/home/desktop/Projects",
  new_volume: true,
}

test("the editor offers a new volume for each container as a default of its own", () => {
  const [draft] = composeDraft({ ...unity, parameters: [workspace] }).steps[0].parameters
  assert.equal(volumeDefault(draft), NEW_VOLUME)
  // It survives the editor and a pasted configuration, and is sent only when set.
  assert.deepEqual(
    composeSteps(composeBody(composeDraft({ ...unity, parameters: [workspace] })))[0].parameters,
    [workspace]
  )
  assert.deepEqual(
    composeSteps(readCompose({ script: "true", parameters: [workspace] }))[0].parameters,
    [workspace]
  )
  // The three defaults: none, a volume by name, and a new one, which clears a named default.
  const none = withVolumeDefault(draft, "")
  assert.deepEqual([none.new_volume, none.default, volumeDefault(none)], [false, "", ""])
  assert.equal("new_volume" in draftParameter(none), false)
  const named = withVolumeDefault(draft, "unity-installs")
  assert.deepEqual([named.new_volume, draftParameter(named).default], [false, "unity-installs"])
  const fresh = withVolumeDefault(named, NEW_VOLUME)
  assert.deepEqual([draftParameter(fresh).new_volume, draftParameter(fresh).default], [true, ""])
  // The editor's select offers both no volume and a new one, whatever the draft holds.
  for (const parameter of [draftParameter(none), draftParameter(fresh)])
    assert.deepEqual(volumeExtras(parameter, { required: false, defaults: true }), {
      none: true,
      fresh: true,
    })
  assert.equal(volumeSelection(draftParameter(fresh), volumeDefault(fresh), false), NEW_VOLUME)
  assert.equal(volumeSelection(draftParameter(none), volumeDefault(none), false), NO_VOLUME)
  // Another type drops it, and only a volume parameter can send it.
  const text = withType(fresh, "text")
  assert.equal(text.new_volume, false)
  assert.equal("new_volume" in draftParameter({ ...text, new_volume: true }), false)
})

test("the create form preselects the new volume, sent empty, and offers none as a dash", () => {
  const compose = { ...unity, parameters: [workspace] }
  // Left alone, the request leaves it empty, which the job turns into the container's volume.
  assert.deepEqual(parameterBody(compose, {}), { UNITY_WORKSPACE: "" })
  assert.equal(volumeSelection(workspace, "", true), NEW_VOLUME)
  // The form offers the new volume, and none too unless a volume is required.
  for (const required of [true, false])
    assert.deepEqual(volumeExtras(workspace, { required, defaults: false }), {
      none: !required,
      fresh: true,
    })
  // None goes as the dash, since an empty value makes the volume, and reads back as none.
  assert.equal(noVolumeValue(workspace, false), WITHOUT_VOLUME)
  assert.equal(volumeSelection(workspace, WITHOUT_VOLUME, false), NO_VOLUME)
  assert.deepEqual(parameterBody(compose, { UNITY_WORKSPACE: WITHOUT_VOLUME }), {
    UNITY_WORKSPACE: "-",
  })
  // The editor's default of none is an empty one without the new volume.
  assert.equal(noVolumeValue(workspace, true), "")
  // A volume picked by name is sent as it is.
  assert.equal(volumeSelection(workspace, "projects", true), "projects")
  assert.deepEqual(parameterBody(compose, { UNITY_WORKSPACE: "projects" }), {
    UNITY_WORKSPACE: "projects",
  })
  // A parameter without it keeps no volume, or nothing chosen yet when one is required.
  const plain = { ...workspace, new_volume: undefined }
  assert.deepEqual(volumeExtras(plain, { required: false, defaults: false }), {
    none: true,
    fresh: false,
  })
  assert.equal(volumeSelection(plain, "", false), NO_VOLUME)
  assert.equal(volumeSelection(plain, "", true), "")
  assert.equal(noVolumeValue(plain, false), "")
})

test("every type survives the editor unchanged", () => {
  const compose = { ...unity, parameters: typed }
  assert.deepEqual(composeBody(composeDraft(compose)), compose)
})

test("a new type starts over without the fields the old one had", () => {
  const [, threads, , , flavor] = composeDraft({ ...unity, parameters: typed }).steps[0].parameters
  const text = withType(threads, "text")
  assert.deepEqual([text.display, text.default], ["input", ""])
  // Bounds stay in the draft, but a text parameter does not send them.
  assert.equal("min" in draftParameter(text), false)
  const many = draftParameter(withType(flavor, "choices"))
  assert.deepEqual([many.display, many.options, many.allow_custom], [undefined, flavors, undefined])
  const secret = withType({ ...composeDraft(unity).steps[0].parameters[1] }, "number")
  assert.equal(draftParameter(secret).secret, false)
  // A choice starts with one empty option to fill in.
  const choice = withType(text, "choice")
  assert.equal(choice.options.length, 1)
  assert.equal(choice.options[0].value, "")
})

test("removing an option drops a default that picked it", () => {
  const [, , , , flavor, toppings] = composeDraft({ ...unity, parameters: typed }).steps[0]
    .parameters
  const vanillaOnly = (draft: typeof flavor) => [draft.options[0]]
  assert.equal(withOptions(toppings, vanillaOnly(toppings)).default, "")
  assert.equal(
    withOptions({ ...toppings, default: "vanilla\nchocolate" }, vanillaOnly(toppings)).default,
    "vanilla"
  )
  // Another value is allowed anyway, so a custom default stays.
  assert.equal(withOptions(flavor, vanillaOnly(flavor)).default, "mint")
  const strict = { ...flavor, allow_custom: false, default: "chocolate" }
  assert.equal(withOptions(strict, vanillaOnly(strict)).default, "")
  assert.equal(withOptions(strict, [...strict.options, newOption()]).default, "chocolate")
})

test("a text value has to match its pattern as a whole before the form sends it", () => {
  const address = "[0-9]{1,3}(\\.[0-9]{1,3}){3}:[0-9]{1,5}"
  assert.equal(patternMatches(address, "192.168.1.20:10080"), true)
  for (const value of ["192.168.1.20", "x192.168.1.20:10080", "192.168.1.20:10080 "])
    assert.equal(patternMatches(address, value), false, value)
  // The whole value, not the first alternative that matches at its start.
  assert.equal(patternMatches("a|ab", "ab"), true)
  assert.equal(patternMatches("a|ab", "abc"), false)
  // An empty value is left to required, and a parameter without a pattern takes anything.
  assert.equal(patternMatches(address, ""), true)
  assert.equal(patternMatches(undefined, "anything"), true)
  assert.equal(patternMatches("", "anything"), true)
})

test("a pattern the browser cannot read is left to the manager", () => {
  // Rust syntax JavaScript does not have, and one that would close the group around it.
  for (const pattern of ["\\A[0-9]+\\z", "(?i)abc", "a)|(b"]) {
    assert.equal(browserPattern(pattern), null, pattern)
    assert.equal(patternMatches(pattern, "a"), undefined, pattern)
  }
  assert.ok(browserPattern("[0-9]+") instanceof RegExp)
})

test("a pattern stays with text parameters through the editor", () => {
  const [, , , , , , accelerator] = composeDraft({ ...unity, parameters: typed }).steps[0]
    .parameters
  assert.deepEqual(draftParameter(accelerator), typed[6])
  // Another type sends no pattern, though the draft keeps it while the type changes.
  const number = withType(accelerator, "number")
  assert.equal("pattern" in draftParameter(number), false)
  assert.equal(draftParameter(withType(number, "text")).pattern, "[^:]+:[0-9]+")
  // A message without a pattern would never be shown, and spaces around it are dropped.
  const cleared = draftParameter({ ...accelerator, pattern: "" })
  assert.equal("pattern" in cleared || "pattern_message" in cleared, false)
  const spaced = draftParameter({ ...accelerator, pattern: " +", pattern_message: " spaces " })
  assert.deepEqual([spaced.pattern, spaced.pattern_message], [" +", "spaces"])
  // A secret can have a pattern too.
  assert.equal(draftParameter({ ...accelerator, secret: true }).pattern, "[^:]+:[0-9]+")
  assert.throws(
    () => readCompose({ script: "true", parameters: [{ name: "A", pattern: 5 }] }),
    /parameter A/
  )
})

test("form values are written the way the script reads them", () => {
  assert.equal(toggleChoice("", "chocolate", true, flavors), "chocolate")
  // Lines follow the options' order, whatever order they were picked in.
  assert.equal(toggleChoice("chocolate", "vanilla", true, flavors), "vanilla\nchocolate")
  assert.equal(toggleChoice("vanilla\nchocolate", "vanilla", false, flavors), "chocolate")
  assert.equal(sliderValue(0.30000000000000004, 0.1), "0.3")
  assert.equal(sliderValue(7, null), "7")
  assert.equal(sliderValue(2.5, 0.25), "2.5")
  // A display the type does not have falls back to the type's own.
  assert.equal(displayOf({ type: "boolean", display: "slider" }), "toggle")
  assert.equal(displayOf({ type: "choice" }), "select")
})

test("a pasted compose keeps typed fields and refuses shapes the manager would", () => {
  const pasted = readCompose({ script: "true", parameters: typed })
  assert.deepEqual(composeSteps(pasted)[0].parameters, typed)
  const cases: [unknown, RegExp][] = [
    [{ name: "A", type: "select" }, /unknown type/],
    [{ name: "A", type: "text", display: "slider" }, /cannot be shown/],
    [{ name: "A", type: "choice", options: [{ label: "x" }] }, /parameter A/],
    [{ name: "A", type: "number", min: "1" }, /parameter A/],
    [{ name: "A", description: 5 }, /parameter A/],
  ]
  for (const [parameter, message] of cases)
    assert.throws(() => readCompose({ script: "true", parameters: [parameter] }), message)
})

/** Two templates merged into one image, each asking for a VERSION of its own. */
const merged: Compose = {
  steps: [
    {
      id: "chrome",
      name: "Google Chrome",
      parameters: [{ ...unity.parameters[2], name: "VERSION", default: "stable" }],
      script: 'echo "$VERSION"',
      root: true,
      timeout_seconds: 60,
    },
    {
      id: "vscode",
      name: "Visual Studio Code",
      parameters: [{ ...unity.parameters[2], name: "VERSION", default: "1.142.0" }],
      script: 'echo "$VERSION"',
      root: false,
      timeout_seconds: 3500,
    },
  ],
}

test("values of several steps go by their step, so two steps may ask for one name", () => {
  assert.deepEqual(
    composeEntries(merged).map((entry) => entry.key),
    ["chrome.VERSION", "vscode.VERSION"]
  )
  // A compose of one step names its values as composes did before steps.
  assert.deepEqual(
    composeEntries(unity).map((entry) => entry.key),
    ["UNITY_EMAIL", "UNITY_PASSWORD", "BRANCH"]
  )
  assert.deepEqual(parameterBody(merged, { "vscode.VERSION": "1.143.0" }), {
    "chrome.VERSION": "stable",
    "vscode.VERSION": "1.143.0",
  })
})

test("a compose of several steps survives the editor and is written as steps", () => {
  assert.deepEqual(composeBody(composeDraft(merged)), merged)
  // One step that names nothing goes back to the form composes had before steps.
  assert.deepEqual(composeBody(composeDraft(unity)), unity)
  const named = { steps: [composeSteps(merged)[0]] }
  assert.deepEqual(composeBody(composeDraft(named)), named)
  assert.deepEqual(readCompose(JSON.parse(JSON.stringify(merged))), merged)
  assert.throws(() => readCompose({ steps: [] }), /steps/)
  assert.throws(() => readCompose({ steps: [{ id: 5, script: "true" }] }), /step/)
  assert.throws(() => readCompose({ steps: [{ id: "a" }] }), /setup script/)
})

test("steps follow the order of the parts they came with", () => {
  const steps = [{ id: "a" }, { id: "own" }, { id: "b" }]
  assert.deepEqual(
    orderSteps(steps, ["b", "a"]).map((step) => step.id),
    ["b", "a", "own"]
  )
})

test("a container's setup values are shown by the step that asked for them", () => {
  const steps = [
    { id: "chrome", name: "Google Chrome", status: "completed" as const },
    { id: "chrome-dev", name: "Chrome Dev", status: "failed" as const },
  ]
  const groups = setupGroups({
    parameters: { "chrome.VERSION": "stable", "chrome-dev.VERSION": "dev" },
    secrets: ["chrome-dev.TOKEN"],
    steps,
  })
  assert.deepEqual(groups, [
    { step: steps[0], values: [["VERSION", "stable"]], secrets: [] },
    { step: steps[1], values: [["VERSION", "dev"]], secrets: ["TOKEN"] },
  ])
  assert.deepEqual(setupGroups({ parameters: { A: "1" }, secrets: ["B"] }), [
    { values: [["A", "1"]], secrets: ["B"] },
  ])
})

const parameter = (patch: Partial<ComposeParameter>): ComposeParameter => ({
  name: "VALUE",
  label: "",
  description: "",
  type: "text",
  required: true,
  secret: false,
  default: "",
  ...patch,
})

test("a step counts the required values it still lacks, as the manager would refuse them", () => {
  const merged: Compose = {
    steps: [
      {
        id: "unity",
        parameters: [
          parameter({ name: "EMAIL" }),
          parameter({ name: "BRANCH", required: false }),
          parameter({ name: "LICENSE", type: "boolean" }),
        ],
        script: "",
        root: false,
        timeout_seconds: 60,
      },
      {
        id: "chrome",
        parameters: [
          // An empty volume parameter that makes one for the container lacks nothing.
          parameter({ name: "PROFILE", type: "volume", new_volume: true, path: "/home/p" }),
          parameter({ name: "CACHE", type: "volume", path: "/home/c" }),
          parameter({ name: "TOOLS", type: "choices" }),
        ],
        script: "",
        root: false,
        timeout_seconds: 60,
      },
    ],
  }
  assert.deepEqual(missingValues(merged, {}), [2, 2])
  assert.deepEqual(
    missingValues(merged, {
      "unity.EMAIL": "ada@example.com",
      "unity.LICENSE": "true",
      "chrome.CACHE": "cache",
      "chrome.TOOLS": "network",
    }),
    [0, 0]
  )
  // A dash still means no volume, which a required parameter refuses.
  assert.deepEqual(
    missingValues(merged, { "chrome.PROFILE": WITHOUT_VOLUME, "chrome.CACHE": "cache" }),
    [2, 2]
  )
  assert.deepEqual(missingValues(unity, { UNITY_EMAIL: "a@b.c" }), [1])
  assert.equal(valueMissing(parameter({ required: false }), ""), false)
})

test("the volumes a compose mounts follow its values, and none for a parameter left without", () => {
  const compose: Compose = {
    steps: [
      {
        id: "re",
        name: "Reverse engineering",
        parameters: [
          parameter({ name: "ANALYSIS", type: "volume", new_volume: true, path: "/home/a" }),
          parameter({ name: "CACHE", type: "volume", required: false, path: "/home/c" }),
        ],
        script: "",
        root: false,
        timeout_seconds: 60,
      },
      {
        id: "chrome",
        parameters: [parameter({ name: "PROFILE", type: "volume", path: "/home/p" })],
        script: "",
        root: false,
        timeout_seconds: 60,
      },
    ],
  }
  assert.deepEqual(setupVolumes(compose, { "chrome.PROFILE": "profiles" }), [
    { volume: null, target: "/home/a", step: "Reverse engineering" },
    { volume: "profiles", target: "/home/p", step: "chrome" },
  ])
  assert.deepEqual(setupVolumes(compose, { "re.ANALYSIS": WITHOUT_VOLUME }), [])
})

test("steps an image kept from the saved image it was built on are named by that image", () => {
  const step = (id: string) => ({ id, parameters: [], root: false, timeout_seconds: 60 })
  const base = {
    image_id: "sha256:base",
    parent_image_id: null,
    name: "Unity desk",
    template_ids: ["unity"],
    compose: { parameters: [], root: false, timeout_seconds: 60 },
  }
  const built = {
    image_id: "sha256:built",
    parent_image_id: "sha256:base",
    name: "Unity and Chrome",
    compose: { steps: [step("unity"), step("chrome")] },
  }
  const records = [built, base]
  assert.deepEqual(inheritedSteps(built, records), { name: "Unity desk", ids: new Set(["unity"]) })
  // A snapshot keeps every step of its image, so none is told apart.
  const snapshot = { ...built, image_id: "sha256:snap", parent_image_id: "sha256:built" }
  assert.equal(inheritedSteps(snapshot, [snapshot, ...records]), null)
  assert.equal(inheritedSteps(base, records), null)
})
