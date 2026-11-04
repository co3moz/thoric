import * as React from "react"
import { flushSync } from "react-dom"
import type { RemixiconComponentType } from "@remixicon/react"
import { cn } from "cn"

import { plural } from "@/lib/i18n"

/** One page of a long form, listed in the form's menu under its group's heading. */
export type FormSection = {
  id: string
  label: string
  icon: RemixiconComponentType
  /** The heading of the group the section is listed under; neighbours with one share it. */
  group: string
  /** How many required values the section still lacks, a count beside its name. */
  missing?: number
  content: React.ReactNode
}

/**
 * A dialog of sections, as wide as they need and of one height, so moving between them does not
 * resize it; on a phone it fills the window.
 */
export const sectionedDialog =
  "flex h-[min(44rem,calc(100dvh-2rem))] flex-col gap-0 p-0 sm:max-w-4xl"

/** The quiet surface of a tile inside a section, such as a switch with its name, without a border. */
export const tile = "rounded-xl bg-input/40 dark:bg-muted/50"

const ShowSection = React.createContext<(id: string) => void>(() => {})

/** Shows another section of the form, such as the one a hint names. */
export const useShowSection = () => React.useContext(ShowSection)

/**
 * A long form as a menu of sections beside the one shown, or a strip of them above it on a narrow
 * screen. Every section stays mounted and only hidden, so the browser still checks every field
 * before the form is sent, and a field it refuses brings its section into view before the browser
 * looks for the field to point at.
 */
export function FormSections({
  sections,
  active,
  onActiveChange,
  label,
  className,
}: {
  sections: FormSection[]
  active: string
  onActiveChange: (id: string) => void
  /** What the menu is called for assistive technology. */
  label: string
  className?: string
}) {
  const id = React.useId()
  // A section that left the form, such as the image setup of another image, gives way to the first.
  const shown = sections.some((section) => section.id === active) ? active : sections[0]?.id
  const tabs = React.useRef(new Map<string, HTMLButtonElement>())
  // One refusal of the form fires an invalid event for every field; the first one decides.
  const pointed = React.useRef(false)
  // The strip of a narrow screen scrolls, so a section shown from elsewhere comes into it.
  React.useEffect(() => {
    tabs.current.get(shown)?.scrollIntoView({ block: "nearest", inline: "nearest" })
  }, [shown])

  const choose = (index: number) => {
    const section = sections[(index + sections.length) % sections.length]
    onActiveChange(section.id)
    tabs.current.get(section.id)?.focus()
  }
  const keys = (event: React.KeyboardEvent, index: number) => {
    const moves: Record<string, number> = {
      ArrowDown: index + 1,
      ArrowRight: index + 1,
      ArrowUp: index - 1,
      ArrowLeft: index - 1,
      Home: 0,
      End: sections.length - 1,
    }
    if (!(event.key in moves)) return
    event.preventDefault()
    choose(moves[event.key])
  }
  const invalid = (event: React.FormEvent) => {
    if (pointed.current) return
    pointed.current = true
    setTimeout(() => (pointed.current = false))
    const panel = (event.target as Element).closest("[data-section]")
    const section = panel?.getAttribute("data-section")
    if (section && section !== shown) flushSync(() => onActiveChange(section))
  }

  return (
    <ShowSection.Provider value={onActiveChange}>
      <div className={cn("flex min-h-0 flex-col sm:flex-row", className)}>
        <div
          role="tablist"
          aria-label={label}
          aria-orientation="vertical"
          className="flex shrink-0 [scrollbar-width:none] gap-1 overflow-x-auto px-6 pb-3 sm:w-56 sm:flex-col sm:gap-0.5 sm:overflow-x-visible sm:overflow-y-auto sm:pr-2 sm:pb-4 sm:pl-3"
        >
          {sections.map((section, index) => {
            const Icon = section.icon
            const selected = section.id === shown
            const heading = section.group !== sections[index - 1]?.group
            return (
              <React.Fragment key={section.id}>
                {heading && (
                  // The grouping is visual; each tab carries its own name.
                  <div
                    aria-hidden
                    className="hidden px-3 pt-4 pb-1 text-xs text-muted-foreground first:pt-1 sm:block"
                  >
                    {section.group}
                  </div>
                )}
                <button
                  ref={(element) => {
                    if (element) tabs.current.set(section.id, element)
                    else tabs.current.delete(section.id)
                  }}
                  type="button"
                  role="tab"
                  id={`${id}-${section.id}-tab`}
                  aria-selected={selected}
                  aria-controls={`${id}-${section.id}`}
                  tabIndex={selected ? 0 : -1}
                  onClick={() => onActiveChange(section.id)}
                  onKeyDown={(event) => keys(event, index)}
                  className="flex h-9 shrink-0 items-center gap-2.5 rounded-xl px-3 text-left text-sm whitespace-nowrap text-muted-foreground outline-none hover:bg-input/40 hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 aria-selected:bg-input/70 aria-selected:font-medium aria-selected:text-foreground dark:hover:bg-muted/50 dark:aria-selected:bg-muted"
                >
                  <Icon className="size-4.5 shrink-0" aria-hidden />
                  <span className="min-w-0 flex-1 truncate">{section.label}</span>
                  {!!section.missing && (
                    <span
                      aria-label={plural(section.missing, {
                        one: "sections.missing.one",
                        other: "sections.missing.other",
                      })}
                      className="flex h-4.5 min-w-4.5 shrink-0 items-center justify-center rounded-full bg-destructive px-1 text-[11px] leading-none font-medium text-background tabular-nums"
                    >
                      {section.missing}
                    </span>
                  )}
                </button>
              </React.Fragment>
            )
          })}
        </div>
        <div
          onInvalidCapture={invalid}
          className="min-h-0 flex-1 overflow-y-auto px-6 pt-0.5 pb-4 sm:pl-4"
        >
          {sections.map((section) => (
            <div
              key={section.id}
              id={`${id}-${section.id}`}
              role="tabpanel"
              aria-labelledby={`${id}-${section.id}-tab`}
              data-section={section.id}
              hidden={section.id !== shown}
              className={cn("flex-col gap-5", section.id === shown ? "flex" : "hidden")}
            >
              <h3 className="text-sm font-medium">{section.label}</h3>
              {section.content}
            </div>
          ))}
        </div>
      </div>
    </ShowSection.Provider>
  )
}
