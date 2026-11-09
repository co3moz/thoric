import * as React from "react"
import { RiLockLine } from "@remixicon/react"

import { CallerBadge, KindBadge, OutcomeBadge } from "@/components/activity-feed"
import { CopyButton } from "@/components/copy-button"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import {
  actionLines,
  activityLabel,
  argumentRows,
  argumentText,
  callCount,
  callDuration,
  exactTime,
  isMasked,
  isTruncated,
  maskedLabel,
  type ActivityEntry,
} from "@/lib/activity"
import { formatNumber, plural, t } from "@/lib/i18n"

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="min-w-0 break-words">{children}</dd>
    </div>
  )
}

/** One argument's value: hidden by its size, cut with its length, or as it is. */
function ArgumentValue({ name, value }: { name: string; value: unknown }) {
  if (isMasked(value))
    return (
      <span className="inline-flex w-fit items-center gap-1.5 rounded-full border border-dashed px-2 py-0.5 text-xs text-muted-foreground">
        <RiLockLine className="size-3.5" aria-hidden />
        {maskedLabel(value)}
      </span>
    )
  const actions = actionLines(value)
  if (actions)
    return (
      <ol className="flex flex-col gap-1 rounded-md border bg-muted/40 px-2.5 py-1.5 font-mono text-xs">
        {actions.map((action, index) => (
          <li key={index} className="flex min-w-0 flex-wrap gap-x-2">
            <span className="font-medium">{action.type}</span>
            {action.fields.map(([key, field]) => (
              <span key={key} className="break-all text-muted-foreground">
                {key} <span className="text-foreground">{field}</span>
              </span>
            ))}
          </li>
        ))}
      </ol>
    )
  const text = argumentText(value)
  // A number, a switch or a short line reads inline; a script or a list gets a box to copy from.
  const inline =
    value === null ||
    ["number", "boolean"].includes(typeof value) ||
    (typeof value === "string" && text.length <= 120 && !text.includes("\n"))
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <div className="flex min-w-0 items-start gap-2">
        {inline ? (
          <code className="font-mono text-xs break-all">{text}</code>
        ) : (
          <pre className="max-h-64 min-w-0 flex-1 overflow-auto rounded-md border bg-muted/40 px-2.5 py-1.5 font-mono text-xs leading-relaxed break-words whitespace-pre-wrap">
            {text}
          </pre>
        )}
        {!inline && <CopyButton size="icon-xs" text={text} label={t("common.copyOf", { name })} />}
      </div>
      {isTruncated(value) && (
        <p className="text-xs text-muted-foreground">
          {t("activity.detail.truncated", {
            shown: formatNumber([...value.$truncated].length),
            total: formatNumber(value.length),
          })}
        </p>
      )}
    </div>
  )
}

/** Everything the log keeps of one call. */
export function ActivityDetail({
  entry,
  containerName,
  containerGone = false,
  onContainer,
  onJob,
}: {
  entry: ActivityEntry
  /** The container's name now, when it still exists. */
  containerName?: string
  /** Whether the container list was read and no longer has the container. */
  containerGone?: boolean
  onContainer?: (id: string) => void
  onJob?: (id: string) => void
}) {
  const rows = argumentRows(entry.arguments)
  const more = typeof entry.arguments.$more === "number" ? entry.arguments.$more : 0
  const container = containerName ?? entry.container_name ?? entry.container_id
  const calls = callCount(entry)
  return (
    <div className="flex flex-col gap-4 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <KindBadge kind={entry.kind} />
        <OutcomeBadge outcome={entry.outcome} />
        <span className="text-xs text-muted-foreground tabular-nums">
          {callDuration(entry.duration_ms)}
        </span>
      </div>
      {calls > 1 && (
        <p className="text-xs text-muted-foreground">{t("activity.detail.repeatsNote")}</p>
      )}
      {entry.error && (
        <Alert variant="destructive">
          <AlertTitle>{t("activity.detail.error")}</AlertTitle>
          <AlertDescription className="break-words whitespace-pre-wrap">
            {entry.error}
          </AlertDescription>
        </Alert>
      )}
      <dl className="grid grid-cols-2 gap-4">
        <Fact label={t("activity.detail.caller")}>
          <CallerBadge entry={entry} />
        </Fact>
        <Fact label={t("activity.detail.credential")}>
          {activityLabel("credential", entry.credential)}
        </Fact>
        <Fact label={t("activity.detail.client")}>
          {entry.client ? (
            <code className="font-mono text-xs">{entry.client}</code>
          ) : (
            <span className="text-muted-foreground">{t("activity.detail.clientUnknown")}</span>
          )}
        </Fact>
        <Fact label={t("activity.detail.started")}>{exactTime(entry.at)}</Fact>
        {calls > 1 && (
          <Fact label={t("activity.detail.calls")}>
            <span className="tabular-nums">{formatNumber(calls)}</span>
          </Fact>
        )}
        {calls > 1 && entry.last_at && (
          <Fact label={t("activity.detail.lastCall")}>{exactTime(entry.last_at)}</Fact>
        )}
        {entry.container_id && (
          <Fact label={t("activity.detail.container")}>
            <span className="flex flex-wrap items-center gap-1.5">
              {containerName && onContainer ? (
                <Button
                  variant="link"
                  className="h-auto p-0 text-foreground"
                  onClick={() => onContainer(entry.container_id!)}
                >
                  {container}
                </Button>
              ) : (
                <span>{container}</span>
              )}
              {containerGone && (
                <Badge variant="outline" className="text-warning">
                  {t("activity.deleted")}
                </Badge>
              )}
            </span>
          </Fact>
        )}
        {entry.server && (
          <Fact label={t("activity.detail.server")}>
            <code className="font-mono text-xs">{entry.server}</code>
          </Fact>
        )}
        {entry.job_id && (
          <Fact label={t("activity.detail.job")}>
            {onJob ? (
              <Button
                variant="link"
                className="h-auto p-0 text-foreground"
                onClick={() => onJob(entry.job_id!)}
              >
                {t("activity.detail.openJob")}
              </Button>
            ) : (
              <code className="font-mono text-xs">{entry.job_id}</code>
            )}
          </Fact>
        )}
      </dl>
      <Separator />
      <div className="flex flex-col gap-3">
        <h3 className="text-xs text-muted-foreground">{t("activity.detail.arguments")}</h3>
        {rows.length ? (
          <dl className="flex flex-col gap-3">
            {rows.map(([name, value]) => (
              <div key={name} className="flex min-w-0 flex-col gap-1">
                <dt className="font-mono text-xs font-medium">{name}</dt>
                <dd className="min-w-0">
                  <ArgumentValue name={name} value={value} />
                </dd>
              </div>
            ))}
          </dl>
        ) : (
          <p className="text-xs text-muted-foreground">{t("activity.detail.noArguments")}</p>
        )}
        {more > 0 && (
          <p className="text-xs text-muted-foreground">
            {plural(more, { one: "activity.more.one", other: "activity.more.other" })}
          </p>
        )}
        <p className="text-xs text-muted-foreground">{t("activity.detail.maskedNote")}</p>
      </div>
    </div>
  )
}
