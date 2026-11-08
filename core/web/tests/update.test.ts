import { describe, expect, test } from "bun:test"

import { build, next, type Answer } from "../src/lib/update"

const status = (current: string, running: boolean): Answer => ({
  kind: "status",
  current,
  running,
})

describe("following an update", () => {
  test("keeps following while the manager is away or the update still runs in it", () => {
    expect(next({ kind: "away" }, "0.6.0", false)).toBe("follow")
    expect(next({ kind: "away" }, "0.6.0", true)).toBe("follow")
    expect(next(status("0.6.0", true), "0.6.0", false)).toBe("follow")
  })

  test("reloads into a new manager however it shows", () => {
    // Another release answers.
    expect(next(status("0.7.0", true), "0.6.0", false)).toBe("reload")
    // A manager that was away answers again, even with the same release, such as latest.
    expect(next(status("0.6.0", true), "0.6.0", true)).toBe("reload")
    // The new manager does not know the page's session.
    expect(next({ kind: "signed-out" }, "0.6.0", false)).toBe("reload")
  })

  test("stops when the update ended in the manager that started it", () => {
    expect(next(status("0.6.0", false), "0.6.0", false)).toBe("done")
  })

  test("tells one edge build from the next by its commit, since both carry one version", () => {
    const before = build({ current: "0.9.3", commit: "1111111111111111111111111111111111111111" })
    const after = build({ current: "0.9.3", commit: "2222222222222222222222222222222222222222" })
    expect(next(status(after, true), before, false)).toBe("reload")
    expect(next(status(before, true), before, false)).toBe("follow")
    // A release has no commit and is told by its version.
    expect(build({ current: "0.9.4", commit: null })).toBe("0.9.4")
  })
})
