import assert from "node:assert/strict"
import { test } from "bun:test"

import {
  changesPath,
  explorerTarget,
  folderPaths,
  splitChangePath,
  type ChangeEntry,
} from "../src/lib/changes"

test("a filter leaves out what the manager takes as its default", () => {
  assert.equal(
    changesPath("c1", { path: "", kinds: ["added", "changed", "deleted"], all: false }, 500),
    "containers/c1/changes?limit=500"
  )
  assert.equal(
    changesPath(
      "c 1",
      { path: "/home/desktop/my project", kinds: ["deleted", "added"], all: true },
      1000
    ),
    "containers/c%201/changes?path=%2Fhome%2Fdesktop%2Fmy+project&kinds=added%2Cdeleted&all=true&limit=1000"
  )
  assert.equal(
    changesPath("c1", { path: "/", kinds: ["changed"], all: false }, 500),
    "containers/c1/changes?kinds=changed&limit=500"
  )
})

test("a path splits into its folder and its name", () => {
  assert.deepEqual(splitChangePath("/home/desktop/notes.txt"), {
    folder: "/home/desktop",
    name: "notes.txt",
  })
  assert.deepEqual(splitChangePath("/etc"), { folder: "/", name: "etc" })
})

test("the explorer opens a folder as itself and a file in its folder, never a deleted one", () => {
  const entries: ChangeEntry[] = [
    { path: "/home/desktop/empty", kind: "added", type: "directory" },
    { path: "/home/desktop/project", kind: "added" },
    { path: "/home/desktop/project/main.py", kind: "added", type: "file" },
    { path: "/home/desktop/projector", kind: "changed" },
    { path: "/etc/legal", kind: "deleted" },
  ]
  const folders = folderPaths(entries)
  // The list holds an entry below it, so it is a folder even without its type; a neighbour that
  // starts with the same letters is not below it.
  assert.deepEqual([...folders], ["/home/desktop/empty", "/home/desktop/project"])
  const [empty, project, main, projector, legal] = entries
  const target = (entry: ChangeEntry) => explorerTarget(entry, folders.has(entry.path))
  assert.deepEqual(target(project), { folder: "/home/desktop/project" })
  assert.deepEqual(target(main), { folder: "/home/desktop/project", name: "main.py" })
  assert.deepEqual(target(empty), { folder: "/home/desktop/empty" })
  assert.deepEqual(target(projector), { folder: "/home/desktop", name: "projector" })
  assert.equal(target(legal), null)
})
