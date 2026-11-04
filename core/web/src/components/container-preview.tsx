import * as React from "react"
import { RiComputerLine } from "@remixicon/react"
import { cn } from "cn"

import { Badge } from "@/components/ui/badge"
import { t } from "@/lib/i18n"
import type { ContainerItem } from "@/lib/types"

const REFRESH = 10_000

/** A number that moves on every `every` milliseconds while the page is in front. */
function useStamp(every: number) {
  const [stamp, setStamp] = React.useState(() => Math.floor(Date.now() / every))
  React.useEffect(() => {
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") setStamp(Math.floor(Date.now() / every))
    }, every)
    return () => clearInterval(timer)
  }, [every])
  return stamp
}

/** The live desktop, letterboxed rather than cropped so the whole screen stays visible. */
export function ContainerPreview({ item, className }: { item: ContainerItem; className?: string }) {
  const { id, spec } = item
  const running = item.state === "running"
  // The picture refreshes every ten seconds while the page is in front; the browser keeps the
  // previous picture on screen until the new one has loaded.
  const stamp = useStamp(REFRESH)
  const [failed, setFailed] = React.useState<number | null>(null)
  const shown = running && failed !== stamp
  // A desktop whose screen did not answer says so until the next picture loads, instead of
  // seeming to load one; a picture that loaded before stays hidden behind it.
  const text = !running
    ? t("containers.stopped")
    : failed === stamp
      ? t("containers.previewUnavailable")
      : t("containers.loadingPreview")

  return (
    <div
      className={cn(
        "relative flex aspect-video shrink-0 items-center justify-center overflow-hidden rounded-md border bg-muted/40",
        className
      )}
    >
      <div className="flex flex-col items-center gap-2 text-xs text-muted-foreground">
        <RiComputerLine className="size-6 opacity-60" />
        {text}
      </div>
      {shown && (
        <img
          alt={t("containers.previewAlt", { name: spec.name })}
          src={`/api/containers/${encodeURIComponent(id)}/screenshot?max_width=640&t=${stamp}`}
          loading="lazy"
          onError={() => setFailed(stamp)}
          className="absolute inset-0 size-full object-contain"
        />
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
