import { describe, expect, test } from "bun:test"
import { readFileSync } from "node:fs"

import { fitMessage, tellFit } from "../src/lib/desktop-fit"

// resolution/fit_tests.rs reads the same messages, so the manager takes what the view sends.
const sent: { on: string; off: string } = JSON.parse(
  readFileSync(new URL("./desktop-fit.json", import.meta.url), "utf8")
)

describe("fitting the desktop to the window", () => {
  test("the view sends the messages the manager takes", () => {
    expect(fitMessage(true)).toBe(sent.on)
    expect(fitMessage(false)).toBe(sent.off)
  })

  test("a socket hears it only once it is open", () => {
    const frames: string[] = []
    const socket = (readyState: number) => ({
      readyState,
      send: (data: string) => void frames.push(data),
    })
    tellFit(socket(WebSocket.CONNECTING), true)
    tellFit(socket(WebSocket.CLOSED), false)
    expect(frames).toEqual([])
    tellFit(socket(WebSocket.OPEN), true)
    expect(frames).toEqual([sent.on])
  })
})
