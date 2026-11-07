import * as React from "react"
import {
  RiArrowRightSLine,
  RiStopCircleLine,
  RiInformationLine,
  RiComputerLine,
  RiMoreLine,
  RiDownloadLine,
  RiStackLine,
  RiUploadLine,
} from "@remixicon/react"
import { cn } from "cn"

import { ConceptIcon } from "@/components/concept"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  SidebarGroup,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  useSidebar,
} from "@/components/ui/sidebar"
import { attempt, containerAction } from "@/lib/actions"
import { t } from "@/lib/i18n"
import { openDialog } from "@/lib/dialogs"
import { useContainers } from "@/lib/queries"
import { href, useRoute } from "@/lib/route"
import { entryCount, runningEntries } from "@/lib/scale"
import type { ContainerItem } from "@/lib/types"

/** How many entries the list shows: a scale set counts once, whatever instances it has. */
const SHOWN = 6

/**
 * Quick access to running containers, where dashboard-01 lists documents, first to last. A scale
 * set's running instances are one entry that opens to them, as Containers opens to its own pages:
 * by a click, or while one of them is on screen.
 */
export function NavContainers() {
  const { isMobile, setOpenMobile } = useSidebar()
  const route = useRoute()
  const running = (useContainers().data ?? []).filter((item) => item.state === "running")
  const entries = runningEntries(running)
  const shown = entries.slice(0, SHOWN)
  const hidden = entryCount(entries) - entryCount(shown)
  const current = route.page === "desktop" ? route.id : undefined
  // A set the user opened or closed stays so; otherwise it is open while one of its desktops is.
  const [toggled, setToggled] = React.useState<Record<string, boolean>>({})
  const close = () => isMobile && setOpenMobile(false)

  return (
    <SidebarGroup className="group-data-[collapsible=icon]:hidden">
      <SidebarGroupLabel>{t("nav.running")}</SidebarGroupLabel>
      <SidebarMenu>
        {running.length === 0 && (
          <SidebarMenuItem>
            <span className="px-2 text-xs text-sidebar-foreground/60">{t("nav.noneRunning")}</span>
          </SidebarMenuItem>
        )}
        {shown.map((entry) => {
          if (entry.kind === "container") {
            const container = entry.item
            return (
              <SidebarMenuItem key={container.id}>
                <SidebarMenuButton
                  asChild
                  isActive={container.id === current}
                  tooltip={t("containers.connectTo", { name: container.spec.name })}
                >
                  <a href={href({ page: "desktop", id: container.id })} onClick={close}>
                    <ConceptIcon concept="container" />
                    <span>{container.spec.name}</span>
                  </a>
                </SidebarMenuButton>
                <ContainerActions container={container} close={close} />
              </SidebarMenuItem>
            )
          }
          const open =
            toggled[entry.set] ?? entry.instances.some((instance) => instance.id === current)
          const listId = `running-set-${entry.set}`
          return (
            <SidebarMenuItem key={`set:${entry.set}`}>
              <SidebarMenuButton
                tooltip={t("scale.setOf", { name: entry.set })}
                aria-expanded={open}
                aria-controls={listId}
                onClick={() => setToggled({ ...toggled, [entry.set]: !open })}
              >
                <ConceptIcon concept="container" icon={RiStackLine} />
                <span className="min-w-0 flex-1 truncate">{entry.set}</span>
                <span
                  className="text-xs text-sidebar-foreground/60 tabular-nums"
                  title={t("nav.instancesRunning", { count: entry.instances.length })}
                >
                  {entry.instances.length}
                </span>
                <RiArrowRightSLine
                  aria-hidden
                  className={cn("transition-transform", open && "rotate-90")}
                />
              </SidebarMenuButton>
              {open && (
                <SidebarMenuSub id={listId}>
                  {entry.instances.map((container) => (
                    <SidebarMenuSubItem key={container.id}>
                      <SidebarMenuSubButton isActive={container.id === current} asChild>
                        <a
                          href={href({ page: "desktop", id: container.id })}
                          onClick={close}
                          title={t("containers.connectTo", { name: container.spec.name })}
                          className="pr-7"
                        >
                          <span>{container.spec.name}</span>
                        </a>
                      </SidebarMenuSubButton>
                      <ContainerActions container={container} close={close} sub />
                    </SidebarMenuSubItem>
                  ))}
                </SidebarMenuSub>
              )}
            </SidebarMenuItem>
          )
        })}
        {hidden > 0 && (
          <SidebarMenuItem>
            <SidebarMenuButton asChild className="text-sidebar-foreground/70">
              <a href={href({ page: "containers" })} onClick={close}>
                <RiMoreLine className="text-sidebar-foreground/70" />
                <span>{t("nav.more", { count: hidden })}</span>
              </a>
            </SidebarMenuButton>
          </SidebarMenuItem>
        )}
      </SidebarMenu>
    </SidebarGroup>
  )
}

/**
 * A running container's actions behind its menu button, shown while the pointer is on its row:
 * a top entry's, or with `sub` an instance's below its set.
 */
function ContainerActions({
  container,
  close,
  sub = false,
}: {
  container: ContainerItem
  close: () => void
  sub?: boolean
}) {
  const { isMobile } = useSidebar()
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <SidebarMenuAction
          showOnHover={!sub}
          className={cn(
            "rounded-sm data-[state=open]:bg-accent",
            // An instance's row is a sub item, which its set's row holds: only its own hover shows it.
            sub &&
              "top-1 group-focus-within/menu-sub-item:opacity-100 group-hover/menu-sub-item:opacity-100 aria-expanded:opacity-100 md:opacity-0"
          )}
        >
          <RiMoreLine />
          <span className="sr-only">
            {t("containers.actionsOf", { name: container.spec.name })}
          </span>
        </SidebarMenuAction>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        className="w-44 rounded-lg"
        side={isMobile ? "bottom" : "right"}
        align={isMobile ? "end" : "start"}
      >
        <DropdownMenuItem tone="success" asChild>
          <a href={href({ page: "desktop", id: container.id })} onClick={close}>
            <RiComputerLine />
            {t("containers.connect")}
          </a>
        </DropdownMenuItem>
        <DropdownMenuItem
          tone="transfer"
          onSelect={() => {
            close()
            openDialog({ kind: "upload", id: container.id })
          }}
        >
          <RiUploadLine />
          {t("uploads.title")}
        </DropdownMenuItem>
        <DropdownMenuItem
          tone="transfer"
          onSelect={() => {
            close()
            openDialog({ kind: "download", id: container.id })
          }}
        >
          <RiDownloadLine />
          {t("downloads.title")}
        </DropdownMenuItem>
        <DropdownMenuItem
          tone="info"
          onSelect={() => {
            close()
            openDialog({ kind: "details", id: container.id })
          }}
        >
          <RiInformationLine />
          {t("containers.details")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          tone="warning"
          onSelect={() => attempt(() => containerAction(container.id, "stop"))}
        >
          <RiStopCircleLine />
          {t("containers.stop")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
