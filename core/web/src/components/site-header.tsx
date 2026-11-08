import { RiArrowRightSLine } from "@remixicon/react"

import { JobsMenu } from "@/components/jobs-menu"
import { ProcessesMenu } from "@/components/processes-menu"
import { Separator } from "@/components/ui/separator"
import { SidebarTrigger } from "@/components/ui/sidebar"

export function SiteHeader({
  title,
  icon,
  parent,
  children,
}: {
  title: string
  icon?: React.ReactNode
  /** The page this one is reached from when the sidebar does not list it. */
  parent?: { title: string; href: string }
  children?: React.ReactNode
}) {
  return (
    <header className="flex h-(--header-height) shrink-0 items-center gap-2 border-b transition-[width,height] ease-linear group-has-data-[collapsible=icon]/sidebar-wrapper:h-(--header-height)">
      <div className="flex w-full items-center gap-1 px-4 lg:gap-2 lg:px-6">
        <SidebarTrigger className="-ml-1" />
        <Separator
          orientation="vertical"
          className="mx-2 data-vertical:h-4 data-vertical:self-center"
        />
        {parent && (
          <>
            <a
              href={parent.href}
              className="hidden rounded-md text-base text-muted-foreground outline-none hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 sm:inline"
            >
              {parent.title}
            </a>
            <RiArrowRightSLine
              aria-hidden
              className="hidden size-4 text-muted-foreground sm:block"
            />
          </>
        )}
        <h1 className="flex min-w-0 items-center gap-2 text-base font-medium [&>svg]:size-4 [&>svg]:shrink-0 [&>svg]:text-muted-foreground">
          {icon}
          <span className="truncate">{title}</span>
        </h1>
        <div className="ml-auto flex items-center gap-3">
          {children}
          <ProcessesMenu />
          <JobsMenu />
        </div>
      </div>
    </header>
  )
}
