import { test } from "bun:test"
import assert from "node:assert/strict"

import {
  aheadText,
  lifecycleBadges,
  lifecycleBody,
  lifecycleDraft,
  lifecycleSummary,
  localTime,
  minutesText,
  NEVER,
  unixTime,
} from "../src/lib/lifecycle"
import type { ContainerItem, Lifecycle, Settings } from "../src/lib/types"

const NOW = 1_800_000_000

const item = (lifecycle: Lifecycle | undefined, extra: Partial<ContainerItem> = {}) =>
  ({
    id: "c1",
    state: "running",
    spec: { name: "Desk", lifecycle },
    idle_stop_at: null,
    ...extra,
  }) as unknown as ContainerItem

test("a time goes into the field and back in the browser's time zone, to the minute", () => {
  const at = NOW - (NOW % 60)
  assert.equal(unixTime(localTime(at)), at)
  assert.match(localTime(at), /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/)
  assert.equal(unixTime(""), null)
  assert.equal(unixTime("not a time"), null)
})

test("a new container starts from the installation's idle stop, an existing one from its own", () => {
  const settings = { defaults: { idle_stop_minutes: 45 } } as unknown as Settings
  assert.deepEqual(lifecycleDraft({}, settings), {
    idle: "45",
    expires: "",
    onExpiry: "stop",
    deleteVolumes: false,
  })
  assert.equal(lifecycleDraft({}).idle, "")
  const own = lifecycleDraft(
    {
      lifecycle: {
        idle_stop_minutes: 0,
        expires_at: NOW,
        on_expiry: "delete",
        delete_volumes: true,
      },
    },
    settings
  )
  assert.deepEqual(own, {
    idle: "",
    expires: localTime(NOW),
    onExpiry: "delete",
    deleteVolumes: true,
  })
})

test("a creation sends the whole policy", () => {
  const draft = {
    idle: "30",
    expires: localTime(NOW),
    onExpiry: "delete" as const,
    deleteVolumes: true,
  }
  assert.deepEqual(lifecycleBody(draft), {
    lifecycle: {
      idle_stop_minutes: 30,
      expires_at: NOW - (NOW % 60),
      on_expiry: "delete",
      delete_volumes: true,
    },
  })
  assert.deepEqual(lifecycleBody(lifecycleDraft({})), { lifecycle: NEVER })
})

test("a change sends only what the form changed, so an expiry an agent moved stays", () => {
  const start = lifecycleDraft({
    lifecycle: {
      idle_stop_minutes: 20,
      expires_at: NOW,
      on_expiry: "delete",
      delete_volumes: true,
    },
  })
  assert.deepEqual(lifecycleBody(start, start), {})
  assert.deepEqual(lifecycleBody({ ...start, idle: "" }, start), {
    lifecycle: { idle_stop_minutes: 0 },
  })
  assert.deepEqual(lifecycleBody({ ...start, expires: "" }, start), {
    lifecycle: { expires_at: null },
  })
  // A stop keeps every volume.
  assert.deepEqual(lifecycleBody({ ...start, onExpiry: "stop" }, start), {
    lifecycle: { on_expiry: "stop", delete_volumes: false },
  })
})

test("lengths of time read short", () => {
  assert.equal(minutesText(30), "30 min")
  assert.equal(minutesText(120), "2 hr")
  assert.equal(minutesText(90), "90 min")
  assert.equal(aheadText(NOW + 12 * 60, NOW), "in 12 min.")
  assert.equal(aheadText(NOW + 5 * 3600, NOW), "in 5 hr.")
  assert.equal(aheadText(NOW + 3 * 86400, NOW), "in 3 days")
  assert.equal(aheadText(NOW - 30, NOW), "in 1 min.")
})

test("a card says how soon an unused desktop stops, and only of a running one", () => {
  const idle = { ...NEVER, idle_stop_minutes: 30 }
  assert.deepEqual(lifecycleBadges(item(undefined), NOW), [])
  assert.deepEqual(lifecycleBadges(item(NEVER), NOW), [])
  const unused = { in_use: false, idle_stop_at: NOW + 12 * 60 }
  const [counting] = lifecycleBadges(item(idle, unused), NOW)
  assert.equal(counting.kind, "idle")
  assert.equal(counting.text, "Idle, stopping in 12 min.")
  // In use, the stop stays a whole idle time away, so nothing counts down.
  const used = { in_use: true, idle_stop_at: NOW + 30 * 60 }
  const [inUse] = lifecycleBadges(item(idle, used), NOW + 10 * 60)
  assert.equal(inUse.text, "Stops after 30 min idle")
  assert.match(inUse.detail, /^In use\./)
  // A manager that does not say whether it is used counts nothing down either.
  assert.equal(lifecycleBadges(item(idle, { idle_stop_at: NOW }), NOW)[0].text, inUse.text)
  const [stopped] = lifecycleBadges(item(idle, { state: "exited", idle_stop_at: NOW }), NOW)
  assert.equal(stopped.text, "Stops after 30 min idle")
  assert.match(stopped.detail, /^While it runs/)
})

test("a card says when the container expires and what happens then", () => {
  const stops = { ...NEVER, expires_at: NOW + 2 * 3600 }
  const [expiry] = lifecycleBadges(item(stops), NOW)
  assert.equal(expiry.kind, "expiry")
  assert.equal(expiry.text, "Expires in 2 hr.")
  assert.match(expiry.detail, /clears its expiry/)
  const deletes = { ...stops, on_expiry: "delete" as const, delete_volumes: true }
  const [deleting] = lifecycleBadges(item(deletes), NOW)
  assert.equal(deleting.text, "Deleting in 2 hr.")
  assert.match(deleting.detail, /with the volumes created for it/)
  const [passed] = lifecycleBadges(item({ ...deletes, expires_at: NOW - 5 }), NOW)
  assert.equal(passed.text, "Expiring now")
})

test("an expiry chosen from the list is sent in minutes, which the manager counts from then", () => {
  const draft = { ...lifecycleDraft({}), expiresIn: "480" }
  assert.deepEqual(lifecycleBody(draft), {
    lifecycle: {
      idle_stop_minutes: 0,
      expires_in_minutes: 480,
      on_expiry: "stop",
      delete_volumes: false,
    },
  })
  const start = lifecycleDraft({
    lifecycle: { idle_stop_minutes: 0, expires_at: NOW, on_expiry: "stop", delete_volumes: false },
  })
  // Never both: the manager refuses a time beside a length.
  assert.deepEqual(lifecycleBody({ ...start, expires: "", expiresIn: "60" }, start), {
    lifecycle: { expires_in_minutes: 60 },
  })
})

test("lengths of a day or more read in days and weeks", () => {
  assert.equal(minutesText(1440), "1 day")
  assert.equal(minutesText(10080), "1 wk")
  assert.equal(minutesText(2880), "2 days")
})

test("the footer says what the lifecycle does, or that it does nothing", () => {
  assert.equal(lifecycleSummary(lifecycleDraft({}), NOW), "Never stops by itself")
  assert.equal(
    lifecycleSummary({ ...lifecycleDraft({}), idle: "30", expiresIn: "480" }, NOW),
    "Stops after 30 min idle · Expires in 8 hr."
  )
})
