import * as React from "react"
import { RiComputerLine } from "@remixicon/react"
import { cn } from "cn"

import { Badge } from "@/components/ui/badge"
import { t } from "@/lib/i18n"
import { previewTurn, usePreviewRefresh } from "@/lib/preview-refresh"
import type { ContainerItem } from "@/lib/types"

/** How long a picture may take before its preview gives the turn back and tries again later. */
const PATIENCE = 30_000

/** Whether the element is on the screen, so a preview scrolled away asks for nothing. */
function useOnScreen(ref: React.RefObject<Element | null>) {
  const [seen, setSeen] = React.useState(false)
  React.useEffect(() => {
    const element = ref.current
    if (!element) return
    const observer = new IntersectionObserver(([entry]) => setSeen(entry.isIntersecting))
    observer.observe(element)
    return () => observer.disconnect()
  }, [ref])
  return seen
}

const inFront = () => document.visibilityState === "visible"
function onVisibility(listener: () => void) {
  document.addEventListener("visibilitychange", listener)
  return () => document.removeEventListener("visibilitychange", listener)
}

/** The picture asked for last, by when it was asked for, and whether it is still on its way. */
type Picture = { stamp: number; loading: boolean; failed: boolean }

function Placeholder({ text }: { text: string }) {
  return (
    <div className="flex flex-col items-center gap-2 text-xs text-muted-foreground">
      <RiComputerLine className="size-6 opacity-60" />
      {text}
    </div>
  )
}

/**
 * A running desktop's screen. The next picture is asked for the chosen time after the last one
 * was, but never before the last one arrived, and only while the preview is on the screen and the
 * page in front, so a slow screenshot never piles up requests; the browser keeps the last picture
 * on screen until the new one has loaded.
 */
function LiveScreen({ id, name }: { id: string; name: string }) {
  const every = usePreviewRefresh()
  const box = React.useRef<HTMLDivElement>(null)
  const seen = useOnScreen(box)
  const front = React.useSyncExternalStore(onVisibility, inFront)
  const [picture, setPicture] = React.useState<Picture | null>(null)
  const turn = React.useRef<(() => void) | null>(null)
  const loading = picture?.loading ?? false

  const giveBack = React.useCallback(() => {
    turn.current?.()
    turn.current = null
  }, [])
  const finish = React.useCallback(
    (ok: boolean) => {
      giveBack()
      setPicture((last) => last && { ...last, loading: false, failed: !ok })
    },
    [giveBack]
  )

  React.useEffect(() => {
    if (!seen || !front || loading) return
    // Off keeps the first picture.
    if (picture && every === 0) return
    const wait = picture ? Math.max(0, picture.stamp + every * 1000 - Date.now()) : 0
    let leave: (() => void) | undefined
    let given = false
    const timer = setTimeout(() => {
      leave = previewTurn((done) => {
        given = true
        turn.current = done
        setPicture({ stamp: Date.now(), loading: true, failed: false })
      })
    }, wait)
    return () => {
      clearTimeout(timer)
      // A turn still waited for is given up; one given is kept until its picture arrives.
      if (!given) leave?.()
    }
  }, [seen, front, loading, picture, every])

  React.useEffect(() => {
    if (!loading) return
    const timer = setTimeout(() => finish(false), PATIENCE)
    return () => clearTimeout(timer)
  }, [loading, finish])

  React.useEffect(() => giveBack, [giveBack])

  // A desktop whose screen did not answer says so until the next picture loads, instead of
  // seeming to load one; a picture that loaded before stays hidden behind it.
  const failed = picture?.failed ?? false
  return (
    <div ref={box} className="absolute inset-0 flex items-center justify-center">
      <Placeholder
        text={failed ? t("containers.previewUnavailable") : t("containers.loadingPreview")}
      />
      {picture && !failed && (
        <img
          alt={t("containers.previewAlt", { name })}
          src={`/api/containers/${encodeURIComponent(id)}/screenshot?max_width=640&t=${picture.stamp}`}
          onLoad={() => finish(true)}
          onError={() => finish(false)}
          className="absolute inset-0 size-full object-contain"
        />
      )}
    </div>
  )
}

/** The live desktop, letterboxed rather than cropped so the whole screen stays visible. */
export function ContainerPreview({ item, className }: { item: ContainerItem; className?: string }) {
  const { id, spec } = item
  return (
    <div
      className={cn(
        "relative flex aspect-video shrink-0 items-center justify-center overflow-hidden rounded-md border bg-muted/40",
        className
      )}
    >
      {item.state === "running" ? (
        <LiveScreen id={id} name={spec.name} />
      ) : (
        <Placeholder text={t("containers.stopped")} />
      )}
      <Badge
        variant="secondary"
        className="absolute top-2 right-2 rounded-sm bg-background/80 font-mono text-[0.7rem] backdrop-blur-sm"
      >
        {spec.resolution}
      </Badge>
    </div>
  )
}
