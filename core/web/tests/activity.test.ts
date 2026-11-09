import { test } from "bun:test"
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"

import {
  actionLines,
  activityLabel,
  activityPath,
  argumentRows,
  argumentText,
  callCount,
  callDuration,
  callerFilter,
  callerLabel,
  dayGroups,
  exportPath,
  filterEndpoint,
  filtersActive,
  isMasked,
  isTruncated,
  maskedLabel,
  relativeTime,
  toolLabel,
} from "../src/lib/activity"

test("a page's path carries each filter the API takes, and no empty one", () => {
  assert.equal(activityPath({}), "activity?limit=50")
  const path = activityPath(
    {
      container: "c1",
      caller: "endpoint:chrome-dev",
      tool: "  exec ",
      errors: true,
      destructive: true,
    },
    1234,
    20
  )
  const query = new URLSearchParams(path.slice("activity?".length))
  assert.deepEqual(Object.fromEntries(query), {
    container: "c1",
    endpoint: "chrome-dev",
    tool: "exec",
    kind: "destructive",
    outcome: "error,refused",
    before: "1234",
    limit: "20",
  })
  // Thoric MCP itself is a caller of its own, not an endpoint.
  const administrator = new URLSearchParams(activityPath({ caller: callerFilter(null) }).slice(9))
  assert.equal(administrator.get("caller"), "administrator")
  assert.equal(administrator.get("endpoint"), null)
  assert.equal(activityPath({ tool: "   " }), "activity?limit=50")
})

test("the API guide documents every parameter a page sends", () => {
  // Rust asserts the route and its parameters are documented; this asserts the page asks for them.
  const api = readFileSync(new URL("../../../docs/API.md", import.meta.url), "utf8")
  assert.ok(api.includes("| GET | `/api/activity` |"))
  const path = activityPath(
    { container: "c", caller: "endpoint:e", tool: "t", errors: true, destructive: true },
    1
  )
  const sent = [...new URLSearchParams(path.slice(9)).keys()]
  for (const name of sent) assert.ok(api.includes(`| \`${name}\` |`), name)
  assert.ok(api.includes("| `caller` |"))
})

test("a caller filter names an endpoint or Thoric MCP", () => {
  assert.equal(callerFilter("chrome-dev"), "endpoint:chrome-dev")
  assert.equal(filterEndpoint("endpoint:chrome-dev"), "chrome-dev")
  assert.equal(filterEndpoint("administrator"), undefined)
  assert.equal(filterEndpoint(undefined), undefined)
  assert.equal(callerLabel({ endpoint: null }), "Thoric MCP")
  assert.equal(callerLabel({ endpoint: "chrome-dev" }), "chrome-dev")
})

test("filters are active once any of them narrows the log", () => {
  assert.equal(filtersActive({}), false)
  assert.equal(filtersActive({ tool: "  " }), false)
  for (const filters of [
    { container: "c1" },
    { caller: "administrator" },
    { tool: "exec" },
    { errors: true },
    { destructive: true },
  ])
    assert.equal(filtersActive(filters), true, JSON.stringify(filters))
})

test("a masked value reads as what it was, and a cut text as its start", () => {
  const hidden = { $masked: "string", length: 27 }
  assert.ok(isMasked(hidden) && !isTruncated(hidden))
  assert.equal(maskedLabel({ $masked: "string", length: 1 }), "1 character, hidden")
  assert.equal(maskedLabel({ $masked: "string", length: 1200 }), "1,200 characters, hidden")
  assert.equal(maskedLabel({ $masked: "array", items: 2 }), "2 items, hidden")
  assert.equal(maskedLabel({ $masked: "object", keys: 1 }), "1 field, hidden")
  assert.equal(maskedLabel({ $masked: "boolean" }), "A value, hidden")
  const cut = { $truncated: "apt-get install", length: 9000 }
  assert.ok(isTruncated(cut) && !isMasked(cut))
  assert.equal(argumentText(cut), "apt-get install…")
  assert.equal(argumentText("rm -rf /tmp/x"), "rm -rf /tmp/x")
  assert.equal(argumentText(hidden), "‹27 characters, hidden›")
  // Inside a list or an object, as JSON with the same words in place of the values.
  assert.equal(
    argumentText([{ type: "type_text", text: { $masked: "string", length: 5 } }]),
    JSON.stringify([{ type: "type_text", text: "‹5 characters, hidden›" }], null, 2)
  )
  assert.equal(argumentText(true), "true")
  assert.ok(!isMasked(null) && !isMasked(["$masked"]) && !isTruncated("x"))
})

test("arguments are listed by name, without the count of those left out", () => {
  assert.deepEqual(
    argumentRows({ path: "/a", $more: 3, data: { $masked: "string", length: 2 } }).map(
      ([key]) => key
    ),
    ["data", "path"]
  )
})

test("times read as how long ago, then as a date", () => {
  const now = Date.UTC(2026, 9, 22, 12, 0, 0)
  assert.equal(relativeTime(now - 10_000, now), "now")
  assert.equal(relativeTime(now - 5 * 60_000, now), "5 min. ago")
  assert.equal(relativeTime(now - 3 * 3600_000, now), "3 hr. ago")
  assert.equal(relativeTime(now - 26 * 3600_000, now), "yesterday")
  assert.match(relativeTime(now - 30 * 86400_000, now), /^Sep \d+$/)
})

test("a call's duration reads in milliseconds, seconds or minutes", () => {
  assert.equal(callDuration(0), "0 ms")
  assert.equal(callDuration(340), "340 ms")
  assert.equal(callDuration(2150), "2.2 s")
  assert.equal(callDuration(65_000), "1m 5s")
})

test("calls are grouped by the local day they began, today and yesterday by name", () => {
  const now = new Date(2026, 9, 22, 15, 0)
  const at = (day: number, hour: number) => new Date(2026, 9, day, hour).getTime()
  const groups = dayGroups(
    [{ at: at(22, 14) }, { at: at(22, 1) }, { at: at(21, 23) }, { at: at(15, 9) }],
    now
  )
  assert.deepEqual(
    groups.map((group) => [group.label, group.entries.length]),
    [
      ["Today", 2],
      ["Yesterday", 1],
      ["Thursday, October 15", 1],
    ]
  )
  // A day of another year names its year.
  const older = dayGroups([{ at: new Date(2025, 0, 2, 9).getTime() }], now)
  assert.match(older[0].label, /2025/)
})

test("kinds, outcomes and credentials have words, and an unknown one keeps its own", () => {
  assert.equal(activityLabel("kind", "destructive"), "Destructive")
  assert.equal(activityLabel("outcome", "refused"), "Refused")
  assert.equal(activityLabel("credential", "inspector"), "Inspector")
  assert.equal(activityLabel("credential", "passkey"), "passkey")
  assert.equal(
    toolLabel({ tool: "browser_navigate", server: "chrome" }),
    "chrome / browser_navigate"
  )
  assert.equal(toolLabel({ tool: "exec_container", server: null }), "exec_container")
})

test("input actions read one a line, with typed text and hidden keys masked", () => {
  const lines = actionLines([
    { type: "click", x: 640, y: 412 },
    { type: "type_text", text: { $masked: "string", length: 14 } },
    { type: "key_press", keys: ["Control", { $masked: "string", length: 1 }] },
  ])
  assert.deepEqual(lines, [
    {
      type: "click",
      fields: [
        ["x", "640"],
        ["y", "412"],
      ],
    },
    { type: "type_text", fields: [["text", "‹14 characters, hidden›"]] },
    { type: "key_press", fields: [["keys", "Control + •"]] },
  ])
  // Anything that is not a list of actions is shown as it is.
  assert.equal(actionLines([]), null)
  assert.equal(actionLines(["a"]), null)
  assert.equal(actionLines({ type: "click" }), null)
  assert.equal(actionLines([{ type: "click" }, { x: 1 }]), null)
})

test("the download follows the page's filters and leaves out its paging", () => {
  assert.equal(exportPath({}, "csv"), "activity/export?format=csv")
  const path = exportPath(
    { container: "c1", caller: "endpoint:set", tool: " get_job ", errors: true, destructive: true },
    "jsonl"
  )
  assert.deepEqual(Object.fromEntries(new URLSearchParams(path.slice("activity/export?".length))), {
    container: "c1",
    endpoint: "set",
    tool: "get_job",
    kind: "destructive",
    outcome: "error,refused",
    format: "jsonl",
  })
  // The guide lists the route and each parameter the page sends to it.
  const api = readFileSync(new URL("../../../docs/API.md", import.meta.url), "utf8")
  assert.ok(api.includes("| GET | `/api/activity/export` |"))
  assert.ok(api.includes("`format`, `csv` (the default) or `jsonl`"))
})

test("a row counts its own call and the reads that repeated it", () => {
  assert.equal(callCount({}), 1)
  assert.equal(callCount({ repeats: 0 }), 1)
  assert.equal(callCount({ repeats: 1000 }), 1001)
})
