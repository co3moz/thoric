import { test } from "bun:test"
import assert from "node:assert/strict"

import type { ContainerSpec, Volume } from "../src/lib/types"
import {
  baseName,
  entryNameError,
  fileDownloadUrl,
  joinPath,
  mountableVolumes,
  ownedVolumes,
  parentPath,
  pathCrumbs,
  sortEntries,
  volumeApi,
  volumeArchiveName,
  volumeArchiveUrl,
  volumeHolders,
  volumeUsers,
  type VolumeEntry,
} from "../src/lib/volume-files"

const spec: ContainerSpec = {
  name: "",
  image: "sha256:ws",
  resolution: "1920x1080",
  gpu: false,
  internet: true,
  audio: false,
  microphone: false,
  sudo: true,
  gamepad: false,
  vpn: false,
  ports: [],
  cpus: null,
  memory_mb: null,
  shm_mb: 512,
  tmp_mb: 512,
  mounts: [],
}

test("volume paths join and climb without leaving the root", () => {
  assert.equal(joinPath("/", "notes.txt"), "/notes.txt")
  assert.equal(joinPath("/project", "notes.txt"), "/project/notes.txt")
  assert.equal(parentPath("/project/assets"), "/project")
  assert.equal(parentPath("/project"), "/")
  assert.equal(parentPath("/"), "/")
  assert.deepEqual(pathCrumbs("/project/assets"), [
    { name: "/", path: "/" },
    { name: "project", path: "/project" },
    { name: "assets", path: "/project/assets" },
  ])
  assert.deepEqual(pathCrumbs("/"), [{ name: "/", path: "/" }])
})

test("a new name is one path component the manager accepts", () => {
  assert.equal(entryNameError("notes.txt"), "")
  assert.equal(entryNameError("Ünïcode name"), "")
  for (const name of ["", ".", "..", "a/b", "a\\b", "tab\tname", "x".repeat(256)])
    assert.notEqual(entryNameError(name), "", JSON.stringify(name))
  // The limit counts bytes, as the file system does.
  assert.notEqual(entryNameError("ğ".repeat(128)), "")
})

test("folders come first, then files by name", () => {
  const entry = (name: string, type: VolumeEntry["type"]): VolumeEntry => ({
    name,
    type,
    size: 0,
    modified_at: 0,
    mode: 0o644,
  })
  const sorted = sortEntries([
    entry("b.txt", "file"),
    entry("src", "directory"),
    entry("a.txt", "file"),
    entry("assets", "directory"),
  ])
  assert.deepEqual(
    sorted.map((item) => item.name),
    ["assets", "src", "a.txt", "b.txt"]
  )
})

test("an archive's file name suggests a volume name the manager accepts", () => {
  assert.equal(volumeArchiveName("unity-projects.tar.gz"), "unity-projects")
  assert.equal(volumeArchiveName("Game Saves (2).TGZ"), "Game-Saves-2-")
  assert.equal(volumeArchiveName("__backup.tar"), "backup")
  // Nothing usable is left for some names; the field then stays empty.
  assert.equal(volumeArchiveName("ü.tar.gz"), "")
  assert.equal(volumeArchiveName("x.tar"), "")
  assert.equal(volumeArchiveUrl("unity-projects"), "/api/volumes/unity-projects/archive")
})

test("the manager's own volumes are not offered for a mount", () => {
  const volume = (Name: string, manager?: boolean): Volume => ({
    Name,
    Driver: "local",
    Scope: "local",
    manager,
  })
  const listed = [volume("thoric-data", true), volume("project", false), volume("saves")]
  // An older manager sends no mark; such a volume is still offered.
  assert.deepEqual(
    mountableVolumes(listed).map((item) => item.Name),
    ["project", "saves"]
  )
})

test("a volume shows where its compose mounts it, and only its owner may delete it along", () => {
  const container = (id: string, mounts: string[], compose: string[]) => ({
    id,
    spec: { ...spec, name: `Desktop ${id}`, mounts: mounts.map(mount) },
    setup: {
      status: "completed" as const,
      image: "i",
      parameters: {},
      secrets: [],
      mounts: compose.map(mount),
    },
  })
  const mount = (volume: string) => ({ volume, target: `/${volume}`, readonly: false })
  const containers = [
    container("e", ["projects"], ["thoric-e-workspace"]),
    container("b", [], ["thoric-e-workspace"]),
  ]
  // The compose's mount counts as much as a volume setting does.
  assert.deepEqual(
    volumeUsers("thoric-e-workspace", containers).map((user) => [user.id, user.compose]),
    [
      ["e", true],
      ["b", true],
    ]
  )
  assert.deepEqual(volumeUsers("projects", containers)[0], {
    id: "e",
    name: "Desktop e",
    target: "/projects",
    readonly: false,
    compose: false,
  })
  // A record without a setup mounts only what its settings say.
  assert.deepEqual(volumeUsers("projects", [{ ...containers[0], setup: null }]).length, 1)
  const owned = (Name: string, container_id?: string) => ({
    Name,
    owner: container_id
      ? { container_id, parameter: "UNITY_WORKSPACE", name: null, deleted: false }
      : null,
  })
  assert.deepEqual(
    ownedVolumes(
      [
        owned("thoric-e-workspace", "e"),
        owned("projects"),
        owned("thoric-e-cache", "e"),
        owned("x", "b"),
      ],
      "e"
    ),
    ["thoric-e-cache", "thoric-e-workspace"]
  )
  // An older manager sends no owner, so nothing is offered for deletion.
  assert.deepEqual(ownedVolumes([{ Name: "thoric-e-workspace" }], "e"), [])
})

test("a volume any container mounts names each of those containers once", () => {
  type Mount = { volume: string; target: string; readonly: boolean }
  const mount = (volume: string, target = `/${volume}`): Mount => ({
    volume,
    target,
    readonly: false,
  })
  const container = (id: string, name: string, mounts: Mount[], compose: Mount[] = []) => ({
    id,
    spec: { ...spec, name, mounts },
    setup: {
      status: "completed" as const,
      image: "i",
      parameters: {},
      secrets: [],
      mounts: compose,
    },
  })
  const containers = [
    // One container that mounts it twice, and two that share a name.
    container("a", "Unity", [mount("projects", "/projects"), mount("projects", "/backup")]),
    container("b", "Build", [], [mount("projects")]),
    container("c", "Build", [mount("projects")]),
    container("d", "Other", [mount("saves")]),
  ]
  assert.deepEqual(volumeHolders("projects", containers), ["Unity", "Build", "Build"])
  assert.deepEqual(volumeHolders("saves", containers), ["Other"])
  // An orphan, such as one whose owner was deleted without it, is held by nobody.
  assert.deepEqual(volumeHolders("thoric-gone-cache", containers), [])
})

test("downloads name the volume and the path in the query", () => {
  const url = new URL(fileDownloadUrl(volumeApi("my volume"), "/a b/c&d.txt"), "http://localhost")
  assert.equal(url.pathname, "/api/volumes/my%20volume/files/download")
  assert.equal(url.searchParams.get("path"), "/a b/c&d.txt")
})

test("a container's files and folders download from the same route", () => {
  const url = new URL(fileDownloadUrl("containers/abc", "/home/desktop/My #1"), "http://localhost")
  assert.equal(url.pathname, "/api/containers/abc/files/download")
  assert.equal(url.searchParams.get("path"), "/home/desktop/My #1")
  // The download is named after the last part, as the manager names its archive.
  assert.equal(baseName("/home/desktop/My #1"), "My #1")
})
