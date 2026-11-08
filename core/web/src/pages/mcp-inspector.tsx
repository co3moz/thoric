import { InspectorView } from "@/components/inspector/inspector-view"
import { DockerAlert, PageBody, SectionHeading } from "@/components/page"
import { t } from "@/lib/i18n"
import { targetGroups } from "@/lib/inspector"
import { openInspector, pickInspectorTarget, useInspectorSession } from "@/lib/inspector-session"
import { useContainers, useMcpEndpoints, useMcpServers, useSettings } from "@/lib/queries"

/** The inspector: one MCP server opened to see what it offers and try it. */
export function McpInspectorPage() {
  const settings = useSettings().data
  const containers = useContainers().data
  const endpoints = useMcpEndpoints()
  const servers = useMcpServers()
  const { client, picked } = useInspectorSession()
  const groups = targetGroups(
    servers.data ?? [],
    endpoints.data ?? [],
    containers ?? [],
    settings?.root_mcp_enabled !== false
  )
  const pickedTarget = groups
    .flatMap((group) => group.choices)
    .find((choice) => choice.key === picked)?.target

  return (
    <PageBody>
      <DockerAlert />
      <SectionHeading title={t("inspector.title")} description={t("inspector.description")} />
      <div className="flex flex-col gap-4 px-4 lg:px-6">
        <InspectorView
          client={client}
          groups={groups}
          containers={containers}
          picked={picked}
          onPick={pickInspectorTarget}
          onConnect={() => pickedTarget && openInspector(pickedTarget, settings?.version ?? "")}
          onDisconnect={() => client?.close()}
        />
      </div>
    </PageBody>
  )
}
