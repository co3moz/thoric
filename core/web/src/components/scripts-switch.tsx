import { Switch } from "@/components/ui/switch"
import { t } from "@/lib/i18n"
import { setShowScripts, useShowScripts } from "@/lib/job-history"

/**
 * Whether the jobs shown include scripts, which an agent runs by the thousand: off, a build or a
 * creation is found among the other jobs. One choice for the Jobs page, the header's menu and
 * the activity chart.
 */
export function ScriptsSwitch() {
  const shown = useShowScripts()
  return (
    <label className="flex shrink-0 items-center gap-2 text-sm">
      <Switch size="sm" checked={shown} onCheckedChange={setShowScripts} />
      {t("jobs.scripts")}
    </label>
  )
}
