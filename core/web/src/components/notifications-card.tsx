import * as React from "react"
import { RiCheckLine, RiErrorWarningLine, RiSendPlaneLine } from "@remixicon/react"
import { toast } from "sonner"

import { StatusBadge } from "@/components/status-badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { api } from "@/lib/api"
import { t } from "@/lib/i18n"
import { useConfiguration } from "@/lib/queries"
import { notifyEvents } from "@/lib/settings"
import type { ConfigVariable, NotifyTest } from "@/lib/types"

const valueOf = (variables: ConfigVariable[], name: string) => {
  const variable = variables.find((entry) => entry.name === name)
  return variable && !variable.secret ? variable.value : null
}

/** Whether notices go anywhere, what they are about and a test that sends one now. */
export function NotificationsCard() {
  const configuration = useConfiguration().data
  const [testing, setTesting] = React.useState(false)
  const [results, setResults] = React.useState<NotifyTest["results"] | null>(null)
  if (!configuration) return <Skeleton className="h-40 rounded-2xl" />
  const url = configuration.variables.find((variable) => variable.name === "NOTIFY_URL")
  const set = url?.secret === true && url.set
  const events = notifyEvents(String(valueOf(configuration.variables, "NOTIFY_EVENTS") ?? ""))
  const linked = Boolean(valueOf(configuration.variables, "DASHBOARD_URL"))

  const test = async () => {
    setTesting(true)
    try {
      const report = await api<NotifyTest>("settings/notify/test", {})
      setResults(report.results)
      const failed = report.results.filter((result) => !result.ok).length
      if (failed) toast.error(t("notify.someFailed", { failed, total: report.results.length }))
      else toast.success(t("notify.allTook"))
    } catch (error) {
      toast.error((error as Error).message)
    } finally {
      setTesting(false)
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("notify.title")}</CardTitle>
        <CardDescription>{set ? t("notify.description") : t("notify.notSet")}</CardDescription>
        <CardAction>
          <StatusBadge
            state={set ? "on" : "off"}
            tone={set ? "success" : "muted"}
            label={set ? t("notify.on") : t("notify.off")}
          />
        </CardAction>
      </CardHeader>
      {set && (
        <CardContent className="flex flex-col gap-4">
          <dl className="flex flex-col gap-0.5">
            <dt className="text-xs text-muted-foreground">{t("notify.sends")}</dt>
            <dd className="font-medium">
              {events
                ? events.map((event) => (event.label ? t(event.label) : event.name)).join(", ")
                : t("notify.nothing")}
            </dd>
          </dl>
          {!linked && <p className="text-sm text-muted-foreground">{t("notify.noLink")}</p>}
          {results && (
            <ul className="flex flex-col gap-2 text-sm" aria-live="polite">
              {results.map((result, index) => (
                <li key={index} className="flex min-w-0 items-start gap-2">
                  {result.ok ? (
                    <RiCheckLine className="mt-0.5 size-4 shrink-0 text-success" />
                  ) : (
                    <RiErrorWarningLine className="mt-0.5 size-4 shrink-0 text-destructive" />
                  )}
                  <div className="flex min-w-0 flex-col">
                    <span className="font-medium">
                      {t("notify.target", {
                        format: t(`notify.format.${result.format}`),
                        host: result.host,
                      })}
                    </span>
                    <span
                      className={
                        result.ok
                          ? "break-words text-muted-foreground"
                          : "break-words text-destructive"
                      }
                    >
                      {result.ok ? t("notify.took", { status: result.status ?? "" }) : result.error}
                    </span>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      )}
      <CardFooter>
        <Button variant="outline" size="sm" disabled={!set || testing} onClick={() => void test()}>
          {testing ? <Spinner data-icon="inline-start" /> : <RiSendPlaneLine />}
          {t("notify.test")}
        </Button>
      </CardFooter>
    </Card>
  )
}
