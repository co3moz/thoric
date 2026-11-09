import * as React from "react"
import {
  RiAlertLine,
  RiCheckLine,
  RiEyeLine,
  RiLinkM,
  RiPencilLine,
  RiRepeat2Line,
  RiShieldKeyholeLine,
  RiUserLine,
} from "@remixicon/react"
import { cn } from "cn"

import { ConceptIcon } from "@/components/concept"
import { Badge } from "@/components/ui/badge"
import {
  activityLabel,
  callCount,
  callDuration,
  callerLabel,
  dayGroups,
  exactTime,
  kindColor,
  relativeTime,
  type ActivityEntry,
  type ActivityKind,
} from "@/lib/activity"
import { formatNumber, t } from "@/lib/i18n"

const kindIcons: Record<ActivityKind, React.ReactNode> = {
  destructive: <RiAlertLine />,
  change: <RiPencilLine />,
  read: <RiEyeLine />,
}

const kindSurface: Record<ActivityKind, string> = {
  destructive: "bg-destructive/10 ring-destructive/30 dark:bg-destructive/20",
  change: "bg-edit/10 ring-edit/30",
  read: "bg-muted ring-border",
}

/**
 * A call's kind as a round mark in its color, named for screen readers. It is opaque, so the
 * timeline's line passes behind it.
 */
export function KindMark({ kind, className }: { kind: ActivityKind; className?: string }) {
  return (
    <span className={cn("flex shrink-0 rounded-full bg-background", className)}>
      <span
        className={cn(
          "flex size-7 items-center justify-center rounded-full ring-1 [&_svg]:size-3.5",
          kindSurface[kind] ?? kindSurface.change,
          kindColor[kind] ?? kindColor.change
        )}
        title={activityLabel("kind", kind)}
      >
        {kindIcons[kind] ?? kindIcons.change}
        <span className="sr-only">{activityLabel("kind", kind)}</span>
      </span>
    </span>
  )
}

/** A call's kind in words beside its icon, for the detail panel. */
export function KindBadge({ kind }: { kind: ActivityKind }) {
  return (
    <Badge variant="outline" className={cn("px-1.5", kindColor[kind] ?? kindColor.change)}>
      {kindIcons[kind] ?? kindIcons.change}
      {activityLabel("kind", kind)}
    </Badge>
  )
}

/** How a call ended: a quiet check, or a red badge for a failure or a refusal. */
export function OutcomeBadge({ outcome }: { outcome: string }) {
  if (outcome === "ok")
    return (
      <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
        <RiCheckLine className="size-3.5 text-success" aria-hidden />
        {activityLabel("outcome", outcome)}
      </span>
    )
  return (
    <Badge variant="destructive" className="px-1.5">
      {activityLabel("outcome", outcome)}
    </Badge>
  )
}

/** Who called: Thoric MCP or an endpoint, marked when it was the inspector or a link. */
export function CallerBadge({ entry }: { entry: Pick<ActivityEntry, "endpoint" | "credential"> }) {
  return (
    <span className="inline-flex flex-wrap items-center gap-1">
      <Badge variant="outline" className="max-w-48 px-1.5 text-muted-foreground">
        {entry.endpoint === null ? <RiShieldKeyholeLine /> : <ConceptIcon concept="mcp" />}
        <span className={cn("truncate", entry.endpoint !== null && "font-mono")}>
          {callerLabel(entry)}
        </span>
      </Badge>
      {entry.credential === "inspector" && (
        <Badge
          variant="outline"
          className="border-info/40 px-1.5 text-info"
          title={t("activity.credentialHint.inspector")}
        >
          <RiUserLine />
          {activityLabel("credential", "inspector")}
        </Badge>
      )}
      {entry.credential === "link" && (
        <Badge variant="outline" className="border-transfer/40 px-1.5 text-transfer">
          <RiLinkM />
          {activityLabel("credential", "link")}
        </Badge>
      )}
    </span>
  )
}

/** The time a call began, relative, with the exact moment on hover. */
export function CallTime({ at, now }: { at: number; now: number }) {
  return (
    <time dateTime={new Date(at).toISOString()} title={exactTime(at)}>
      {relativeTime(at, now)}
    </time>
  )
}

/** The current time, moving on every 30 seconds so relative times stay true. */
export function useNow() {
  const [now, setNow] = React.useState(() => Date.now())
  React.useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000)
    return () => clearInterval(timer)
  }, [])
  return now
}

/** One call: its kind's mark on the timeline, the tool, who called, where and how it ended. */
function ActivityRow({
  entry,
  now,
  showContainer,
  containerName,
  onSelect,
}: {
  entry: ActivityEntry
  now: number
  showContainer: boolean
  containerName?: string
  onSelect: (entry: ActivityEntry) => void
}) {
  const container = containerName ?? entry.container_name ?? entry.container_id?.slice(0, 8)
  const calls = callCount(entry)
  return (
    <li className="relative before:absolute before:top-0 before:bottom-0 before:left-[1.625rem] before:w-px before:bg-border first:before:top-4 last:before:bottom-auto last:before:h-4 only:before:hidden">
      <button
        type="button"
        onClick={() => onSelect(entry)}
        className="relative grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-x-3 rounded-xl px-3 py-2 text-left transition-colors outline-none hover:bg-muted/60 focus-visible:ring-[3px] focus-visible:ring-ring/50"
      >
        <KindMark kind={entry.kind} />
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
            <span className="min-w-0 font-mono text-sm font-medium break-words">
              {entry.server && (
                <span className="text-muted-foreground">
                  {entry.server}
                  <span aria-hidden> / </span>
                </span>
              )}
              {entry.tool}
            </span>
            {calls > 1 && (
              <Badge
                variant="secondary"
                className="px-1.5 tabular-nums"
                title={
                  entry.last_at
                    ? t("activity.repeatedUntil", { time: exactTime(entry.last_at) })
                    : undefined
                }
              >
                <RiRepeat2Line />
                {t("activity.calls", { count: formatNumber(calls) })}
              </Badge>
            )}
            <CallerBadge entry={entry} />
            {showContainer && container && (
              <span className="inline-flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
                <ConceptIcon concept="container" className="size-3.5 shrink-0" />
                <span className="truncate">{container}</span>
              </span>
            )}
          </div>
          {entry.summary && (
            <code className="truncate font-mono text-xs text-muted-foreground">
              {entry.summary}
            </code>
          )}
          {entry.error && (
            <p className="line-clamp-2 text-xs break-words text-destructive">{entry.error}</p>
          )}
        </div>
        <div className="flex flex-col items-end gap-1 text-xs whitespace-nowrap text-muted-foreground tabular-nums">
          <CallTime at={entry.at} now={now} />
          <span className="flex items-center gap-2">
            {entry.outcome !== "ok" && <OutcomeBadge outcome={entry.outcome} />}
            <span>{callDuration(entry.duration_ms)}</span>
          </span>
        </div>
      </button>
    </li>
  )
}

/**
 * Calls on a timeline, newest first: grouped by day with each day's name above it, or as one
 * plain list for a short excerpt.
 */
export function ActivityFeed({
  entries,
  onSelect,
  showContainer = true,
  containerNames,
  grouped = true,
}: {
  entries: ActivityEntry[]
  onSelect: (entry: ActivityEntry) => void
  showContainer?: boolean
  /** The containers' current names, by ID. */
  containerNames?: Map<string, string>
  grouped?: boolean
}) {
  const now = useNow()
  const rows = (list: ActivityEntry[]) => (
    <ul className="flex flex-col">
      {list.map((entry) => (
        <ActivityRow
          key={entry.id}
          entry={entry}
          now={now}
          showContainer={showContainer}
          containerName={entry.container_id ? containerNames?.get(entry.container_id) : undefined}
          onSelect={onSelect}
        />
      ))}
    </ul>
  )
  if (!grouped) return rows(entries)
  return (
    <div className="flex flex-col gap-4">
      {dayGroups(entries, new Date(now)).map((group) => (
        <section key={group.key} className="flex flex-col gap-1" aria-label={group.label}>
          <h3 className="sticky top-0 z-10 bg-background/90 px-3 py-1 text-xs font-medium text-muted-foreground backdrop-blur-sm">
            {group.label}
          </h3>
          {rows(group.entries)}
        </section>
      ))}
    </div>
  )
}
