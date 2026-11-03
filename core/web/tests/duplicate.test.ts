import { test } from "bun:test"
import assert from "node:assert/strict"

import { duplicateName, duplicateVolumes, nameTaken } from "../src/lib/duplicate"
import { pendingContainers, pendingImages } from "../src/lib/pending"
import type { Container, Job, Volume } from "../src/lib/types"

const named = (...names: string[]) =>
  names.map((name) => ({ spec: { name } }) as Pick<Container, "spec">)

test("a duplicate is offered a name no container has, as the manager compares names", () => {
  assert.equal(duplicateName("Editor", named("Editor")), "Editor-copy")
  // The manager compares without case and the spaces around a name.
  assert.equal(duplicateName(" Editor ", named("Editor", "editor-COPY ")), "Editor-copy-2")
  assert.equal(
    duplicateName("Editor", named("Editor", "Editor-copy", "Editor-copy-2")),
    "Editor-copy-3"
  )
  // A long name is cut so the suggestion stays within the manager's 100 characters.
  const long = duplicateName("x".repeat(100), [])
  assert.equal(long.length, 100)
  assert.ok(long.endsWith("-copy"))
  assert.ok(nameTaken(" EDITOR", named("Editor")))
  assert.ok(!nameTaken("Editor 2", named("Editor")))
})

test("the volumes created for a container are copied and the others shared", () => {
  const container = {
    id: "orig",
    spec: {
      mounts: [
        { volume: "assets", target: "/home/desktop/Assets", readonly: true },
        { volume: "thoric-orig-cache", target: "/home/desktop/.cache", readonly: false },
      ],
    },
    setup: {
      mounts: [
        { volume: "thoric-orig-workspace", target: "/home/desktop/Projects", readonly: false },
        { volume: "assets", target: "/opt/assets", readonly: false },
      ],
    },
  } as unknown as Pick<Container, "id" | "spec" | "setup">
  const owner = (container_id: string) => ({
    container_id,
    parameter: "WORKSPACE",
    name: null,
    deleted: false,
  })
  const volumes: Pick<Volume, "Name" | "owner">[] = [
    { Name: "assets", owner: null },
    { Name: "thoric-orig-workspace", owner: owner("orig") },
    { Name: "thoric-orig-cache", owner: owner("orig") },
    { Name: "thoric-other-workspace", owner: owner("other") },
  ]
  const { copied, shared } = duplicateVolumes(container, volumes)
  assert.deepEqual(
    copied.map((mount) => mount.volume),
    ["thoric-orig-cache", "thoric-orig-workspace"]
  )
  // Each volume once, at the first folder it is mounted at.
  assert.deepEqual(shared, [{ volume: "assets", target: "/home/desktop/Assets", readonly: true }])
  // Without the volume list nothing is known to be the container's own.
  assert.equal(duplicateVolumes(container, []).copied.length, 0)
})

test("a duplicate shows as a pending container and its snapshot as a pending image", () => {
  const job: Job = {
    id: "j1",
    kind: "duplicate",
    resource: "container:new",
    name: "Editor-copy",
    status: "running",
    created_at: 1,
    finished_at: null,
    log: "",
  }
  assert.deepEqual(
    pendingContainers([job], []).map(({ id, name }) => [id, name]),
    [["new", "Editor-copy"]]
  )
  // The snapshot's record is saved under the job's ID, with the duplicate's name.
  assert.deepEqual(
    pendingImages([job], []).map(({ id, name, kind }) => [id, name, kind]),
    [["j1", "Editor-copy", "snapshot"]]
  )
  assert.equal(pendingImages([job], [{ id: "j1" }]).length, 0)
})
