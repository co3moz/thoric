import { test } from "bun:test"
import assert from "node:assert/strict"

import { staleKeys } from "../src/lib/events"
import { pendingContainers } from "../src/lib/pending"
import {
  bySet,
  entryCount,
  room,
  runningEntries,
  samePolicies,
  scalingBody,
  scalingDraft,
  scalingError,
  scalingTargets,
  serverPolicies,
  setNameOf,
  setServerNames,
  setServers,
  type ScaleSet,
} from "../src/lib/scale"
import type { ContainerItem, Job } from "../src/lib/types"

const set = (name: string, instances: number, max = 4) =>
  ({
    name,
    max,
    min: 1,
    instances: Array.from({ length: instances }, (_, index) => ({
      number: index + 1,
      container_id: `${name}-${index + 1}`,
    })),
  }) as unknown as ScaleSet

const container = (id: string, scale?: { set: string; number: number }) =>
  ({ id, spec: { name: id }, scale: scale ?? null }) as unknown as ContainerItem

test("a set is named after its container as the manager names it", () => {
  assert.equal(setNameOf("Reverse engineering"), "reverse-engineering")
  assert.equal(setNameOf("  re1 "), "re1")
  assert.equal(setNameOf("!!!"), "set")
  assert.equal(setNameOf("x".repeat(60)).length, 40)
  assert.ok(!setNameOf(`${"a".repeat(39)} b`).endsWith("-"))
})

test("the scaling numbers are checked before they are sent", () => {
  const draft = { ...scalingDraft(), enabled: true }
  assert.equal(scalingError(draft), null)
  assert.equal(scalingError({ ...draft, max: "0" }), "max")
  assert.equal(scalingError({ ...draft, max: "17" }), "max")
  assert.equal(scalingError({ ...draft, min: "5", max: "4" }), "min")
  assert.equal(scalingError({ ...draft, min: "1.5" }), "min")
  assert.equal(scalingError({ ...draft, idle: "10081" }), "idle")
})

test("a request sends the policies that are not shared, and the set's name only when typed", () => {
  const body = scalingBody({
    ...scalingDraft(),
    enabled: true,
    min: "2",
    max: "6",
    idle: "30",
    volumes: { "/home/desktop/analysis": "shared", "/home/desktop/.config/chrome-agent": "new" },
  })
  assert.deepEqual(body, {
    min: 2,
    max: 6,
    idle_delete_minutes: 30,
    volumes: { "/home/desktop/.config/chrome-agent": "new" },
  })
  assert.equal(scalingBody({ ...scalingDraft(), set: " re " }).set, "re")
  assert.deepEqual(scalingTargets([{ target: "/a" }, { target: "/b" }, { target: "/a" }]), [
    "/a",
    "/b",
  ])
  // A server every instance runs is the default and goes unsaid.
  assert.deepEqual(
    scalingBody({ ...scalingDraft(), servers: { ghidra: "first", chrome: "each" } }).servers,
    { ghidra: "first" }
  )
  assert.equal(scalingBody({ ...scalingDraft(), servers: { chrome: "each" } }).servers, undefined)
})

test("a set's server policies start from the set and compare without the default", () => {
  const pinned = { ...set("re", 2), servers: { ghidra: "first" as const } }
  assert.deepEqual(scalingDraft(pinned).servers, { ghidra: "first" })
  assert.deepEqual(serverPolicies({ ghidra: "first", chrome: "each" }), { ghidra: "first" })
  assert.ok(samePolicies({ ghidra: "first", chrome: "each" }, { ghidra: "first" }))
  assert.ok(samePolicies({}, { chrome: "each" }))
  assert.ok(!samePolicies({ ghidra: "first" }, {}))
})

test("a set's servers are listed once, a server instance 1 runs for all from instance 1", () => {
  const record = (container_id: string, name: string) => ({ container_id, name })
  // Instance 2 comes first in the list, as the manager sorts containers by name.
  const servers = [
    record("re-2", "chrome"),
    record("re-2", "ghidra"),
    record("re-1", "ghidra"),
    record("re-1", "chrome"),
    record("other", "db"),
  ]
  const each = set("re", 2)
  assert.deepEqual(setServers(each, servers), [record("re-1", "chrome"), record("re-1", "ghidra")])
  // Instance 1 is gone: a server every instance runs comes from instance 2, one instance 1 runs
  // alone from nowhere, though its name stays for its policy.
  const pinned = {
    ...each,
    servers: { ghidra: "first" as const },
    instances: each.instances.slice(1),
  }
  assert.deepEqual(setServers(pinned, servers), [record("re-2", "chrome")])
  assert.deepEqual(setServerNames(pinned, servers), ["chrome", "ghidra"])
  assert.deepEqual(setServers(undefined, servers), [])
})

test("the containers page shows each set's instances in order and the others after them", () => {
  const sets = [set("re", 2), set("empty", 0)]
  const items = [
    container("b"),
    container("re-2", { set: "re", number: 2 }),
    container("re-1", { set: "re", number: 1 }),
    // An instance of a set the page does not know yet is shown on its own.
    container("x-1", { set: "x", number: 1 }),
  ]
  const { groups, alone } = bySet(items, sets)
  assert.deepEqual(
    groups.map((group) => [group.set.name, group.instances.map((item) => item.id)]),
    [["re", ["re-1", "re-2"]]]
  )
  assert.deepEqual(
    alone.map((item) => item.id),
    ["b", "x-1"]
  )
  assert.equal(room(set("re", 2)), 2)
  assert.equal(room(set("re", 4)), 0)
})

test("an instance's creation is a pending container and its end refreshes the sets", () => {
  const job = {
    id: "j1",
    kind: "scale-up",
    resource: "container:c9",
    name: "re-3",
    status: "running",
    created_at: 1,
    finished_at: null,
    log: "",
    result: null,
  } as Job
  assert.deepEqual(
    pendingContainers([job], []).map((entry) => entry.name),
    ["re-3"]
  )
  for (const kind of ["scale-up", "scale-down", "idle-delete", "scale-set-delete"])
    assert.ok(
      staleKeys({ kind, resource: "container:c9", status: "completed" }).includes("scale-sets")
    )
})

test("the sidebar lists running containers first to last, a set's instances under it", () => {
  const item = (id: string, created_at: number, scale?: { set: string; number: number }) =>
    ({ id, created_at, spec: { name: id }, scale: scale ?? null }) as unknown as ContainerItem
  const entries = runningEntries([
    item("re-4", 40, { set: "re", number: 4 }),
    item("late", 50),
    item("re-2", 20, { set: "re", number: 2 }),
    item("re", 10, { set: "re", number: 1 }),
    item("early", 5),
    item("other-1", 30, { set: "other", number: 1 }),
  ])
  assert.deepEqual(
    entries.map((entry) =>
      entry.kind === "set"
        ? `${entry.set}: ${entry.instances.map((instance) => instance.id).join(" ")}`
        : entry.item.id
    ),
    ["early", "re: re re-2 re-4", "other: other-1", "late"]
  )
  assert.equal(entryCount(entries), 6)
  assert.equal(entryCount(entries.slice(0, 2)), 4)
})
