import { test } from "bun:test"
import assert from "node:assert/strict"

import { staleKeys } from "../src/lib/events"
import { pendingContainers } from "../src/lib/pending"
import {
  bySet,
  room,
  scalingBody,
  scalingDraft,
  scalingError,
  scalingTargets,
  setNameOf,
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
