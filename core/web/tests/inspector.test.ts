import { test } from "bun:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"

import {
  asJson,
  base64Bytes,
  expandTemplate,
  fieldChoices,
  filterItems,
  inputSchemaOf,
  mediaSource,
  sameJson,
  shortenStrings,
  stringValues,
  targetGroups,
  targetKey,
  templateVariables,
  toolHints,
  type ToolItem,
} from "../src/lib/inspector"
import { INSPECT_PATH } from "../src/lib/mcp-client"
import type { JsonSchema } from "../src/lib/schema-form"
import type { ContainerItem, DesktopServer, McpEndpoint } from "../src/lib/types"

test("a URI template names its variables once, without modifiers", () => {
  assert.deepEqual(templateVariables("repo://{owner}/{repo}/tree{/path*}{?ref,depth:3}"), [
    "owner",
    "repo",
    "path",
    "ref",
    "depth",
  ])
})

test("a URI template fills in as RFC 6570 says", () => {
  const values = { owner: "co3moz", repo: "a b", path: '["src","lib"]', ref: "main", q: "" }
  assert.equal(expandTemplate("repo://{owner}/{repo}", values), "repo://co3moz/a%20b")
  assert.equal(expandTemplate("file:///{+path}", { path: "a/b c" }), "file:///a/b%20c")
  assert.equal(expandTemplate("x{/path*}", values), "x/src/lib")
  assert.equal(expandTemplate("x{?ref,missing}", values), "x?ref=main")
  assert.equal(expandTemplate("x{?q}", values), "x?q=")
  assert.equal(expandTemplate("x{?ref}{&owner}", values), "x?ref=main&owner=co3moz")
  assert.equal(expandTemplate("x/{owner:3}", values), "x/co3")
  assert.equal(expandTemplate("x{#owner}", values), "x#co3moz")
  // A variable left out takes its punctuation with it.
  assert.equal(expandTemplate("x{/missing}{.missing}", values), "x")
  assert.equal(expandTemplate("q/{owner}", { owner: "it's!" }), "q/it%27s%21")
})

test("the search keeps items by name, title or URI, in the order chosen", () => {
  const items = [
    { uri: "file:///b.txt", name: "beta" },
    { uri: "file:///a.txt", name: "Alpha", title: "First file" },
    { uri: "db://c", name: "gamma" },
  ]
  assert.deepEqual(
    filterItems("resource", items, "file", "server").map((item) => item.name),
    ["beta", "Alpha"]
  )
  assert.deepEqual(
    filterItems("resource", items, "first", "server").map((item) => item.name),
    ["Alpha"]
  )
  assert.deepEqual(
    filterItems("resource", items, "", "ascending").map((item) => item.name),
    ["Alpha", "beta", "gamma"]
  )
  assert.deepEqual(
    filterItems("resource", items, "", "descending").map((item) => item.name),
    ["gamma", "beta", "Alpha"]
  )
  // The server's own list stays in its order.
  assert.equal(items[0].name, "beta")
})

test("prompts and templates take their arguments as required strings", () => {
  const prompt = inputSchemaOf("prompt", {
    name: "review",
    arguments: [{ name: "code", description: "The code", required: true }, { name: "style" }],
  })
  assert.deepEqual(prompt.required, ["code"])
  assert.equal(prompt.properties?.code.description, "The code")
  const template = inputSchemaOf("template", { uriTemplate: "logs://{app}/{day}" })
  assert.deepEqual(template.required, ["app", "day"])
  assert.deepEqual(inputSchemaOf("tool", { name: "t" }), { type: "object" })
  assert.deepEqual(stringValues({ a: "x", b: 2, c: null, d: [1] }), { a: "x", b: "2", d: "[1]" })
})

test("a text block is told to repeat structuredContent whatever the order of its keys", () => {
  assert.ok(sameJson('{"b":[1,{"y":2,"x":1}],"a":null}', { a: null, b: [1, { x: 1, y: 2 }] }))
  assert.ok(!sameJson('{"a":1}', { a: 2 }))
  assert.ok(!sameJson("not json", {}))
  assert.equal(asJson(' {"a":1} '), '{\n  "a": 1\n}')
  assert.equal(asJson("12"), null)
  assert.equal(asJson("{broken"), null)
})

test("long strings are shortened for showing, and media is only plain base64", () => {
  const long = "A".repeat(5000)
  const short = shortenStrings({ data: long, list: [long], keep: "short" }) as Record<
    string,
    unknown
  >
  assert.ok((short.data as string).length < 200)
  assert.ok((short.list as string[])[0].length < 200)
  assert.equal(short.keep, "short")
  assert.equal(mediaSource("image/png", "iVBO\nRw=="), "data:image/png;base64,iVBORw==")
  assert.equal(mediaSource('image/png" onerror=', "AAAA"), null)
  assert.equal(mediaSource("image/png", "<script>"), null)
  assert.equal(mediaSource(undefined, "AAAA"), null)
  assert.equal(base64Bytes("AAAA"), 3)
  assert.equal(base64Bytes("AAA="), 2)
  assert.equal(base64Bytes("AA=="), 1)
})

test("a tool's claims become hints, destructive only when it is not read-only", () => {
  const tool = (annotations: ToolItem["annotations"], outputSchema?: JsonSchema): ToolItem => ({
    name: "t",
    annotations,
    ...(outputSchema ? { outputSchema } : {}),
  })
  assert.deepEqual(toolHints(tool({ readOnlyHint: true, destructiveHint: true })), ["readOnly"])
  assert.deepEqual(toolHints(tool({ destructiveHint: true, openWorldHint: true }, {})), [
    "destructive",
    "openWorld",
    "structured",
  ])
  assert.deepEqual(toolHints(tool(undefined)), [])
})

const server = (container: string, name: string, extra: Partial<DesktopServer> = {}) =>
  ({
    container_id: container,
    container_name: container,
    name,
    command: "npx",
    url: null,
    error: null,
    ...extra,
  }) as DesktopServer
const container = (id: string, state: string) =>
  ({ id, state, spec: { name: `Desktop ${id}` } }) as Pick<ContainerItem, "id" | "state" | "spec">

test("the picker offers what can be opened, and says why the rest cannot", () => {
  const groups = targetGroups(
    [
      server("c1", "zeta"),
      server("c1", "alpha"),
      server("c2", "chrome"),
      server("c1", "broken", { command: null, error: "broken.json: expected value" }),
    ],
    [{ name: "b", path: "/mcp/b" } as McpEndpoint, { name: "a", path: "/mcp/a" } as McpEndpoint],
    [container("c1", "running"), container("c2", "exited")],
    true
  )
  assert.deepEqual(
    groups.map((group) => group.label),
    ["Thoric", "Endpoints", "Desktop c1", "Desktop c2"]
  )
  // A container's servers sit under its icon and color, the endpoints under MCP's.
  assert.deepEqual(
    groups.map((group) => group.concept),
    [undefined, "mcp", "container", "container"]
  )
  assert.deepEqual(
    groups[1].choices.map((choice) => choice.label),
    ["/mcp/a", "/mcp/b"]
  )
  const first = groups[2].choices
  assert.deepEqual(
    first.map((choice) => [choice.label, choice.unavailable]),
    [
      ["alpha", undefined],
      ["broken", "broken.json: expected value"],
      ["zeta", undefined],
    ]
  )
  assert.ok(groups[3].choices[0].unavailable, "a stopped container's server")
  assert.equal(first[0].key, "server:c1:alpha")
  assert.deepEqual(first[0].target, { kind: "server", container: "c1", name: "alpha" })

  // With the root /mcp off, only it is unavailable: an endpoint answers either way.
  const off = targetGroups([], [{ name: "a", path: "/mcp/a" } as McpEndpoint], [], false)
  assert.ok(off[0].choices[0].unavailable)
  assert.equal(off[1].choices[0].unavailable, undefined)
  assert.equal(targetKey({ kind: "endpoint", name: "a" }), "endpoint:a")
})

test("Thoric's own MCP offers its containers by name for container_id, and no one else does", () => {
  const containers = [container("c2", "exited"), container("c1", "running")]
  const choices = fieldChoices({ kind: "administrator" }, containers).get("container_id")
  assert.ok(choices)
  assert.equal(choices.concept, "container")
  // By name, but the field takes the Thoric ID.
  assert.deepEqual(
    choices.items.map((item) => [item.value, item.label]),
    [
      ["c1", "Desktop c1"],
      ["c2", "Desktop c2"],
    ]
  )
  assert.ok(
    choices.items.every((item) => item.detail),
    "each says its state"
  )
  assert.equal(fieldChoices({ kind: "administrator" }, []).get("container_id")?.items.length, 0)
  // An endpoint names its containers in an enum, and a desktop server's container_id is its own.
  assert.equal(fieldChoices({ kind: "endpoint", name: "a" }, containers).size, 0)
  assert.equal(fieldChoices({ kind: "server", container: "c1", name: "x" }, containers).size, 0)
  // A name an object inherits is not a field's choices.
  assert.equal(fieldChoices({ kind: "administrator" }, containers).get("constructor"), undefined)
})

test("the inspector's socket is a route the API guide lists", () => {
  // Rust asserts the route is documented; this asserts the page asks for that route.
  const api = readFileSync(new URL("../../../docs/API.md", import.meta.url), "utf8")
  assert.ok(api.includes(`\`/api/${INSPECT_PATH}\``), INSPECT_PATH)
})
