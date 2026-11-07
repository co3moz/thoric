import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { test } from "bun:test"

import {
  childrenOf,
  compareProcesses,
  matchesProcess,
  needsRoot,
  nextSort,
  processesPath,
  processRows,
  rowOrder,
  runningContainers,
  signallable,
  type ProcessInfo,
  type ProcessList,
} from "../src/lib/processes"
import type { ContainerItem } from "../src/lib/types"
import type { UsageSeries } from "../src/lib/usage"

const process = (pid: number, ppid: number, values: Partial<ProcessInfo> = {}): ProcessInfo => ({
  pid,
  ppid,
  uid: 1001,
  user: "desktop",
  state: "S",
  name: `p${pid}`,
  command: `/usr/bin/p${pid}`,
  role: "program",
  threads: 1,
  rss_bytes: 0,
  cpu_percent: 0,
  cpu_seconds: 0,
  started_at: null,
  ...values,
})

// 1 starts the session, which runs a browser with two renderers and a terminal; 50 came through
// docker exec.
const desktop = [
  process(1, 0, { name: "desktop-session", role: "desktop" }),
  process(9, 1, { name: "Xtigervnc", role: "desktop", cpu_percent: 2, rss_bytes: 100 }),
  process(20, 1, { name: "chrome", cpu_percent: 10, rss_bytes: 500 }),
  process(21, 20, {
    name: "chrome",
    command: "chrome --type=renderer",
    cpu_percent: 30,
    rss_bytes: 300,
  }),
  process(22, 20, { name: "chrome", command: "chrome --type=gpu", cpu_percent: 5, rss_bytes: 200 }),
  process(30, 1, { name: "bash", user: "root", uid: 0 }),
  process(50, 0, { name: "python3", role: "thoric", rss_bytes: 50 }),
]

const options = {
  sort: { key: "cpu" as const, descending: true },
  query: "",
  tree: false,
  folded: new Set<number>(),
}

test("the page asks for as many processes as the manager lists", () => {
  assert.equal(processesPath("c 1"), "containers/c%201/processes?limit=1000")
})

test("a heading sorts its own way first and the other way when clicked again", () => {
  assert.deepEqual(nextSort({ key: "name", descending: false }, "cpu"), {
    key: "cpu",
    descending: true,
  })
  assert.deepEqual(nextSort({ key: "cpu", descending: true }, "pid"), {
    key: "pid",
    descending: false,
  })
  assert.deepEqual(nextSort({ key: "pid", descending: false }, "pid"), {
    key: "pid",
    descending: true,
  })
  const byMemory = [...desktop].sort(compareProcesses({ key: "memory", descending: true }))
  assert.deepEqual(
    byMemory.map((item) => item.pid),
    [20, 21, 22, 9, 50, 1, 30]
  )
  const byName = [...desktop].sort(compareProcesses({ key: "name", descending: false }))
  // Processes of one name keep the order of their PIDs.
  assert.deepEqual(
    byName.map((item) => item.pid),
    [30, 20, 21, 22, 1, 50, 9]
  )
})

test("a search matches the name, the command line, the user or the PID", () => {
  const chrome = desktop[3]
  assert.ok(matchesProcess(chrome, "CHROME"))
  assert.ok(matchesProcess(chrome, "renderer"))
  assert.ok(matchesProcess(chrome, " desktop "))
  assert.ok(matchesProcess(chrome, "21"))
  assert.ok(!matchesProcess(chrome, "2"))
  assert.ok(!matchesProcess(chrome, "gpu"))
  assert.ok(matchesProcess(chrome, ""))
})

test("the flat list is the matching processes, sorted, each on its own", () => {
  const rows = processRows(desktop, options)
  assert.deepEqual(
    rows.map((row) => row.process.pid),
    [21, 20, 22, 9, 50, 1, 30]
  )
  assert.ok(rows.every((row) => row.depth === 0 && row.children === 0))
  assert.deepEqual(rows[1].totals, { cpu: 10, rss: 500, count: 1 })
  assert.deepEqual(
    processRows(desktop, { ...options, query: "chrome" }).map((row) => row.process.pid),
    [21, 20, 22]
  )
})

test("the tree puts each process under its parent with what its descendants use", () => {
  const rows = processRows(desktop, { ...options, tree: true })
  assert.deepEqual(
    rows.map((row) => [row.process.pid, row.depth]),
    [
      [1, 0],
      [20, 1],
      [21, 2],
      [22, 2],
      [9, 1],
      [30, 1],
      [50, 0],
    ]
  )
  const chrome = rows.find((row) => row.process.pid === 20)!
  assert.equal(chrome.children, 2)
  assert.deepEqual(chrome.totals, { cpu: 45, rss: 1000, count: 3 })
  assert.deepEqual(rows[0].totals, { cpu: 47, rss: 1100, count: 6 })
})

test("a folded process hides what is below it until a search looks for it", () => {
  const folded = new Set([20])
  assert.deepEqual(
    processRows(desktop, { ...options, tree: true, folded }).map((row) => row.process.pid),
    [1, 20, 9, 30, 50]
  )
  // A search shows each match with the processes above it, and only those.
  assert.deepEqual(
    processRows(desktop, { ...options, tree: true, folded, query: "renderer" }).map((row) => [
      row.process.pid,
      row.depth,
    ]),
    [
      [1, 0],
      [20, 1],
      [21, 2],
    ]
  )
})

test("held rows keep their places while their figures change", () => {
  const before = processRows(desktop, options)
  const held = rowOrder(before)
  // The renderer goes quiet, Xtigervnc gets busy and a process starts.
  const after = desktop.map((item) =>
    item.pid === 21
      ? { ...item, cpu_percent: 0 }
      : item.pid === 9
        ? { ...item, cpu_percent: 90 }
        : item
  )
  after.push(process(70, 1, { cpu_percent: 99 }))
  const rows = processRows(after, { ...options, held })
  assert.deepEqual(
    rows.map((row) => row.process.pid),
    [...before.map((row) => row.process.pid), 70]
  )
  assert.equal(rows[0].process.cpu_percent, 0)
  // Unheld, they sort again.
  assert.deepEqual(
    processRows(after, options)
      .slice(0, 3)
      .map((row) => row.process.pid),
    [70, 9, 20]
  )
  // In the tree, siblings keep theirs and stay under their parents.
  const tree = { ...options, tree: true }
  const heldTree = rowOrder(processRows(desktop, tree))
  assert.deepEqual(
    processRows(after, { ...tree, held: heldTree }).map((row) => [row.process.pid, row.depth]),
    [
      [1, 0],
      [20, 1],
      [21, 2],
      [22, 2],
      [9, 1],
      [30, 1],
      [70, 1],
      [50, 0],
    ]
  )
})

test("a process whose parent is not listed is a root of the tree", () => {
  const listed = [process(7, 3), process(8, 7), process(9, 9)]
  const children = childrenOf(listed)
  assert.deepEqual(
    children.get(0)?.map((item) => item.pid),
    [7, 9]
  )
  assert.deepEqual(
    children.get(7)?.map((item) => item.pid),
    [8]
  )
  assert.deepEqual(
    processRows(listed, { ...options, tree: true }).map((row) => row.process.pid),
    [7, 8, 9]
  )
})

test("another user's process is signalled as root and PID 1 not at all", () => {
  assert.ok(!needsRoot(desktop[2]))
  assert.ok(needsRoot(desktop[5]))
  assert.ok(!signallable(desktop[0]))
  assert.ok(signallable(desktop[1]))
})

test("the header lists the running containers, the busiest first", () => {
  const container = (id: string, name: string, state: string) =>
    ({ id, spec: { name }, state }) as unknown as ContainerItem
  const series = (id: string, t: number, cpu: number) =>
    ({ id, samples: [{ t, cpu, memory: cpu * 1000 }] }) as unknown as UsageSeries
  const running = runningContainers(
    [
      container("a", "alpha", "running"),
      container("b", "beta", "running"),
      container("c", "gamma", "exited"),
      container("d", "delta", "running"),
      container("e", "epsilon", "running"),
    ],
    [
      series("a", 10_000, 0.5),
      series("b", 10_000, 2),
      series("c", 10_000, 9),
      series("e", 1_000, 4),
    ],
    10_000
  )
  assert.deepEqual(
    running.map((item) => [item.name, item.cpu, item.memory]),
    [
      ["beta", 2, 2000],
      ["alpha", 0.5, 500],
      // Without a recent sample, by name.
      ["delta", null, null],
      ["epsilon", null, null],
    ]
  )
})

test("the manager's process list reads as the page's types", () => {
  const fixture: ProcessList = JSON.parse(
    readFileSync(new URL("../../manager/tests/process-list.json", import.meta.url), "utf8")
  )
  const keys = Object.keys(desktop[0]).sort()
  for (const item of fixture.processes) assert.deepEqual(Object.keys(item).sort(), keys)
  assert.deepEqual(
    processRows(fixture.processes, options).map((row) => row.process.name),
    ["chrome", "Xtigervnc", "python3"]
  )
  assert.deepEqual(Object.keys(fixture).sort(), [
    "cpu_percent",
    "cpus",
    "interval_ms",
    "matched",
    "memory_bytes",
    "memory_limit_bytes",
    "processes",
    "rss_bytes",
    "total",
  ])
})
