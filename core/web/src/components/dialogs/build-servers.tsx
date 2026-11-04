import { RiDeleteBinLine } from "@remixicon/react"
import { cn } from "cn"

import { ConceptIcon } from "@/components/concept"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { FieldDescription, FieldSet } from "@/components/ui/field"
import { t } from "@/lib/i18n"
import { serverCommand, serverDetails } from "@/lib/mcp"
import type { ServerDeclaration } from "@/lib/types"
import { tile } from "./form-sections"

/**
 * The desktop MCP servers a build writes into its image, shown before anything is built like its
 * script: how each starts, what it is given, and a way to leave one out.
 */
export function BuildServers({
  servers,
  onRemove,
}: {
  servers: ServerDeclaration[]
  onRemove: (name: string) => void
}) {
  // The section's heading names the list.
  return (
    <FieldSet aria-label={t("build.mcp")} className="gap-3">
      <FieldDescription className="text-xs">{t("build.mcpHint")}</FieldDescription>
      <ul className="flex flex-col gap-2">
        {servers.map((server) => {
          const details = serverDetails(
            Object.keys(server.env ?? {}),
            Object.keys(server.headers ?? {}),
            server.cwd
          )
          return (
            <li
              key={server.name}
              className={cn(
                "flex flex-wrap items-start justify-between gap-x-3 gap-y-2 px-4 py-3",
                tile
              )}
            >
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <div className="flex flex-wrap items-center gap-2">
                  <ConceptIcon concept="mcp" className="size-4 shrink-0" />
                  <span className="font-mono text-sm font-medium break-all">{server.name}</span>
                  <Badge variant="outline" className="text-muted-foreground">
                    {server.command ? "stdio" : "HTTP"}
                  </Badge>
                </div>
                {server.description && (
                  <p className="text-xs text-muted-foreground">{server.description}</p>
                )}
                <code className="font-mono text-[0.7rem] break-all text-muted-foreground">
                  {serverCommand(server)}
                </code>
                {details && <p className="text-xs break-words text-muted-foreground">{details}</p>}
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="text-destructive"
                onClick={() => onRemove(server.name)}
                aria-label={t("build.mcpRemoveOf", { name: server.name })}
              >
                <RiDeleteBinLine data-icon="inline-start" />
                {t("build.partRemove")}
              </Button>
            </li>
          )
        })}
      </ul>
    </FieldSet>
  )
}
