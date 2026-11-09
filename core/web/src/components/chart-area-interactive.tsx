import * as React from "react"
import { Area, AreaChart, CartesianGrid, XAxis, YAxis } from "recharts"

import { ScriptsSwitch } from "@/components/scripts-switch"
import { useIsMobile } from "@/hooks/use-mobile"
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from "@/components/ui/chart"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { jobActivity } from "@/lib/format"
import { formatDate, plural, t } from "@/lib/i18n"
import { useShowScripts } from "@/lib/job-history"
import { useJobHistory } from "@/lib/queries"

const chartConfig = () =>
  ({
    completed: { label: t("state.completed"), color: "var(--primary)" },
    failed: { label: t("chart.failed"), color: "var(--destructive)" },
  }) satisfies ChartConfig

const ranges = () => [
  { value: "90", label: t("chart.range.90") },
  { value: "30", label: t("chart.range.30") },
  { value: "7", label: t("chart.range.7") },
]

// The range as it reads inside the summary sentence.
const during = {
  90: "chart.during.90",
  30: "chart.during.30",
  7: "chart.during.7",
} as const

const day = (value: string) =>
  formatDate(new Date(`${value}T00:00:00`).getTime() / 1000, { month: "short", day: "numeric" })

/**
 * Finished jobs per day: how busy the manager has been and whether operations succeed. The
 * manager counts every job that ends, so the range is whole however many jobs it still keeps.
 */
export function ChartAreaInteractive() {
  const isMobile = useIsMobile()
  const [range, setRange] = React.useState("30")
  const scripts = useShowScripts()
  const days = isMobile ? 7 : Number(range)
  const hours = useJobHistory(days).data?.hours
  const data = React.useMemo(() => jobActivity(hours ?? [], days, scripts), [hours, days, scripts])
  const total = data.reduce((sum, point) => sum + point.completed + point.failed, 0)
  const options = ranges()
  const selected = options.find((item) => item.value === String(days))?.label ?? options[2].label

  return (
    <Card className="@container/card">
      <CardHeader>
        <CardTitle>{t("chart.title")}</CardTitle>
        <CardDescription>
          <span className="hidden @[540px]/card:block">
            {plural(
              total,
              { one: "chart.summary.one", other: "chart.summary.other" },
              { range: t(during[days as keyof typeof during] ?? "chart.during.7") }
            )}
          </span>
          <span className="@[540px]/card:hidden">{selected}</span>
        </CardDescription>
        <CardAction className="flex flex-wrap items-center justify-end gap-x-4 gap-y-2">
          <ScriptsSwitch />
          <ToggleGroup
            type="single"
            value={String(days)}
            onValueChange={(value) => value && setRange(value)}
            variant="outline"
            className="hidden *:data-[slot=toggle-group-item]:px-4! @[767px]/card:flex"
          >
            {options.map((item) => (
              <ToggleGroupItem key={item.value} value={item.value}>
                {item.label}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          <Select value={String(days)} onValueChange={setRange}>
            <SelectTrigger
              className="flex w-40 **:data-[slot=select-value]:block **:data-[slot=select-value]:truncate @[767px]/card:hidden"
              size="sm"
              aria-label={t("chart.timeRange")}
            >
              <SelectValue placeholder={options[1].label} />
            </SelectTrigger>
            <SelectContent className="rounded-xl">
              {options.map((item) => (
                <SelectItem key={item.value} value={item.value} className="rounded-lg">
                  {item.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </CardAction>
      </CardHeader>
      <CardContent className="px-2 pt-4 sm:px-6 sm:pt-6">
        <ChartContainer config={chartConfig()} className="aspect-auto h-[250px] w-full">
          <AreaChart data={data} margin={{ left: 0, right: 8 }}>
            <defs>
              <linearGradient id="fillCompleted" x1="0" y1="0" x2="0" y2="1">
                <stop offset="5%" stopColor="var(--color-completed)" stopOpacity={0.8} />
                <stop offset="95%" stopColor="var(--color-completed)" stopOpacity={0.1} />
              </linearGradient>
              <linearGradient id="fillFailed" x1="0" y1="0" x2="0" y2="1">
                <stop offset="5%" stopColor="var(--color-failed)" stopOpacity={0.8} />
                <stop offset="95%" stopColor="var(--color-failed)" stopOpacity={0.1} />
              </linearGradient>
            </defs>
            <CartesianGrid vertical={false} />
            <XAxis
              dataKey="date"
              tickLine={false}
              axisLine={false}
              tickMargin={8}
              minTickGap={32}
              tickFormatter={day}
            />
            <YAxis allowDecimals={false} tickLine={false} axisLine={false} width={28} />
            <ChartTooltip
              cursor={false}
              content={
                <ChartTooltipContent
                  labelFormatter={(value) => day(String(value))}
                  indicator="dot"
                />
              }
            />
            <Area
              dataKey="failed"
              type="monotone"
              fill="url(#fillFailed)"
              stroke="var(--color-failed)"
              stackId="a"
            />
            <Area
              dataKey="completed"
              type="monotone"
              fill="url(#fillCompleted)"
              stroke="var(--color-completed)"
              stackId="a"
            />
            <ChartLegend content={<ChartLegendContent />} />
          </AreaChart>
        </ChartContainer>
      </CardContent>
    </Card>
  )
}
