import { expect, test } from "bun:test"
import { readFileSync } from "node:fs"

import {
  isScript,
  jobsPath,
  OTHER_JOBS,
  parseShowScripts,
  SCRIPT_KINDS,
} from "../src/lib/job-history"

test("scripts are shown unless the stored choice hides them", () => {
  expect(parseShowScripts("hidden")).toBe(false)
  for (const value of [null, "", "shown", "false", "0"]) expect(parseShowScripts(value)).toBe(true)
})

test("the list without scripts asks the manager for it, under a key of its own", () => {
  expect(jobsPath(true)).toBe("jobs")
  expect(jobsPath(false)).toBe("jobs?scripts=false")
  expect(OTHER_JOBS).not.toBe("jobs")
  expect(isScript({ kind: "exec" })).toBe(true)
  expect(isScript({ kind: "build" })).toBe(false)
})

test("the script kinds are the manager's", () => {
  const manager = readFileSync(
    new URL("../../manager/src/jobs/history.rs", import.meta.url),
    "utf8"
  )
  const kinds = SCRIPT_KINDS.map((kind) => JSON.stringify(kind)).join(", ")
  expect(manager).toContain(`pub(crate) const SCRIPT_KINDS: &[&str] = &[${kinds}];`)
})
