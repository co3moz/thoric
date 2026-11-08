import { describe, expect, test } from "bun:test"

import { parseRefresh, turns } from "../src/lib/preview-refresh"

describe("the preview refresh choice", () => {
  test("a stored choice is read back, anything else is the default", () => {
    expect(parseRefresh("1")).toBe(1)
    expect(parseRefresh("0")).toBe(0)
    expect(parseRefresh("30")).toBe(30)
    for (const value of [null, "", "3", "-1", "1.5", "10s", "Infinity"]) {
      expect(parseRefresh(value)).toBe(10)
    }
  })
})

describe("preview turns", () => {
  test("no more than the size hold a turn, and the next in line gets one given back", () => {
    const take = turns(2)
    const held: Array<() => void> = []
    const got: number[] = []
    for (const index of [0, 1, 2, 3]) {
      take((done) => {
        got.push(index)
        held[index] = done
      })
    }
    expect(got).toEqual([0, 1])
    held[0]()
    expect(got).toEqual([0, 1, 2])
    // Giving a turn back twice frees only one.
    held[0]()
    expect(got).toEqual([0, 1, 2])
    held[1]()
    held[2]()
    expect(got).toEqual([0, 1, 2, 3])
  })

  test("leaving the queue never takes a turn, and leaving with one gives it back", () => {
    const take = turns(1)
    const got: string[] = []
    const first = take(() => got.push("first"))
    const second = take(() => got.push("second"))
    take(() => got.push("third"))
    second()
    first()
    expect(got).toEqual(["first", "third"])
    // A turn left after it was given, once more, frees nothing more than it held.
    first()
    take(() => got.push("fourth"))
    expect(got).toEqual(["first", "third"])
  })
})
