import * as React from "react"
import RFB from "@novnc/novnc"
import {
  RiArrowLeftLine,
  RiAspectRatioLine,
  RiPlayCircleLine,
  RiClipboardLine,
  RiFullscreenLine,
  RiMicLine,
  RiMicOffLine,
  RiFullscreenExitLine,
  RiRestartLine,
  RiDownloadLine,
  RiUploadLine,
  RiVolumeUpLine,
  RiVolumeMuteLine,
} from "@remixicon/react"
import { toast } from "sonner"
import { cn } from "cn"

import { HumanRequestStrip } from "@/components/human-requests"
import { JobsMenu } from "@/components/jobs-menu"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty"
import { Separator } from "@/components/ui/separator"
import { Spinner } from "@/components/ui/spinner"
import { Textarea } from "@/components/ui/textarea"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useFileDrop } from "@/hooks/use-file-drop"
import { attempt, containerAction } from "@/lib/actions"
import { wsUrl } from "@/lib/api"
import { tellFit } from "@/lib/desktop-fit"
import { openDialog } from "@/lib/dialogs"
import { t } from "@/lib/i18n"
import { DesktopMedia, type MediaState } from "@/lib/media"
import { useContainers } from "@/lib/queries"
import { href } from "@/lib/route"
import { holdUnload } from "@/lib/unload"

type Connection = "connecting" | "connected" | "disconnected" | "failed"
const connectionText = (connection: Connection) => t(`desktop.connection.${connection}`)

/**
 * What covers the screen until it shows: a spinner while it connects, and once the connection
 * failed or closed, why, with **Reconnect**, rather than a black screen.
 */
function ConnectionOverlay({
  connection,
  onReconnect,
}: {
  connection: Exclude<Connection, "connected">
  onReconnect: () => void
}) {
  if (connection === "connecting")
    return (
      <div className="pointer-events-none absolute inset-0 flex items-center justify-center gap-2 text-sm text-white/70">
        <Spinner />
        {connectionText(connection)}
      </div>
    )
  return (
    <Empty className="absolute inset-0 bg-black/70 text-white/80">
      <EmptyHeader>
        <EmptyTitle className="text-base">{connectionText(connection)}</EmptyTitle>
        <EmptyDescription className="text-white/70">
          {t(`desktop.lost.${connection}`)}
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button onClick={onReconnect}>
          <RiRestartLine data-icon="inline-start" />
          {t("desktop.reconnect")}
        </Button>
      </EmptyContent>
    </Empty>
  )
}

/**
 * A compact toolbar button: an icon with its label on wide screens and a tooltip on narrow ones,
 * narrower still on a phone so every tool fits beside the name. A `hint` says more in the
 * tooltip at every width; such a button shows its label from 1536 px, since the tooltip names
 * it anyway and every tool's word would not fit at 1280.
 */
function ToolButton({
  label,
  hint,
  icon,
  className,
  ...props
}: React.ComponentProps<typeof Button> & { label: string; hint?: string; icon: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          aria-label={label}
          className={cn("max-sm:px-1", className)}
          {...props}
        >
          {icon}
          <span className={hint ? "hidden 2xl:inline" : "hidden xl:inline"}>{label}</span>
        </Button>
      </TooltipTrigger>
      {hint ? (
        <TooltipContent>
          <div className="flex flex-col gap-0.5">
            <span className="font-medium 2xl:hidden">{label}</span>
            <span>{hint}</span>
          </div>
        </TooltipContent>
      ) : (
        <TooltipContent className="xl:hidden">{label}</TooltipContent>
      )}
    </Tooltip>
  )
}

type KeyboardLock = { lock: (codes?: string[]) => Promise<void>; unlock: () => void }

/**
 * The Keyboard Lock API, which only Chromium has. While it holds in fullscreen, keys the browser
 * keeps for itself, such as Ctrl+W, reach the desktop, and holding Escape leaves fullscreen.
 */
function keyboardLock() {
  const keyboard = (navigator as Navigator & { keyboard?: Partial<KeyboardLock> }).keyboard
  return keyboard?.lock && keyboard.unlock ? (keyboard as KeyboardLock) : null
}

const RESIZE_KEY = "thoric.desktop.resize"

/**
 * Whether the desktop takes this window's size, a choice kept in this browser only. It is off
 * unless chosen: a new size reaches the agents and the other viewers of the desktop too. The size
 * lasts while the choice is on and the view connected; then the manager gives the desktop the
 * resolution of its settings again.
 */
function storedResize() {
  try {
    return localStorage.getItem(RESIZE_KEY) === "true"
  } catch {
    return false
  }
}

function storeResize(on: boolean) {
  try {
    if (on) localStorage.setItem(RESIZE_KEY, "true")
    else localStorage.removeItem(RESIZE_KEY)
  } catch {
    // The choice is a convenience; without storage the view scales the desktop next time.
  }
}

function useMedia() {
  const [state, setState] = React.useState<MediaState>({
    output: false,
    input: false,
    outputPending: false,
    inputPending: false,
  })
  const [media] = React.useState(
    () =>
      new DesktopMedia({
        createContext: () => new AudioContext({ sampleRate: 48000 }),
        createNode: (ctx, name, options) => new AudioWorkletNode(ctx, name, options),
        createSocket: (url) => new WebSocket(url),
        url: wsUrl,
        // Browsers offer the microphone only on HTTPS or localhost.
        getUserMedia: navigator.mediaDevices?.getUserMedia?.bind(navigator.mediaDevices),
        onError: (message) => toast.error(message),
        onState: setState,
      })
  )
  React.useEffect(() => () => media.close(), [media])
  return { media, state }
}

/** The remote desktop over VNC, with audio, microphone, clipboard and uploads. */
export function DesktopPage({ id }: { id: string }) {
  const containers = useContainers(false)
  const item = containers.data?.find((entry) => entry.id === id)
  const running = item?.state === "running"
  // A stopped desktop is watched until it runs, so starting it here connects.
  useContainers(!!item && !running)

  const view = React.useRef<HTMLElement>(null)
  const screen = React.useRef<HTMLDivElement>(null)
  const rfb = React.useRef<RFB | null>(null)
  // The current connection's socket, on which the manager also hears whether this window fits.
  const channel = React.useRef<WebSocket | null>(null)
  const [generation, setGeneration] = React.useState(0)
  // Each connection attempt reports under its own key; a new attempt starts out connecting.
  const attemptKey = `${id}:${generation}:${running}`
  const [status, setStatus] = React.useState<{ key: string; value: Connection } | null>(null)
  const connection = status?.key === attemptKey ? status.value : "connecting"
  const [remoteClipboard, setRemoteClipboard] = React.useState("")
  const [clipboard, setClipboard] = React.useState<string | null>(null)
  const [fullscreen, setFullscreen] = React.useState(false)
  const [resize, setResize] = React.useState(storedResize)
  // What a socket that opens later tells the manager.
  const fits = React.useRef(resize)
  const { media, state: audio } = useMedia()
  const drop = useFileDrop((files) => openDialog({ kind: "upload", id, files }))

  React.useEffect(() => {
    if (!running || !screen.current) return
    media.select(id)
    const report = (value: Connection) => setStatus({ key: attemptKey, value })
    let current: RFB
    let socket: WebSocket | null = null
    try {
      // The view opens the socket itself, so it can speak to the manager beside noVNC.
      socket = new WebSocket(wsUrl(`containers/${encodeURIComponent(id)}/vnc`))
      const opened = socket
      // Heard before noVNC asks for a size, so the manager knows whose size the screen takes.
      opened.addEventListener("open", () => tellFit(opened, fits.current))
      current = new RFB(screen.current, opened)
    } catch (error) {
      socket?.close()
      toast.error((error as Error).message)
      // Reported like the connection's own events, after the effect has run.
      queueMicrotask(() => report("failed"))
      return
    }
    rfb.current = current
    channel.current = socket
    // The view scales the desktop to fit; its resolution changes only when the toggle asks.
    current.scaleViewport = true
    current.showDotCursor = true
    current.focusOnClick = true
    // Events from a connection that was replaced or closed must not touch the current one.
    const live = () => rfb.current === current
    current.addEventListener("connect", () => live() && report("connected"))
    current.addEventListener("disconnect", (event) => {
      if (!live()) return
      media.close()
      report((event as CustomEvent<{ clean: boolean }>).detail.clean ? "disconnected" : "failed")
    })
    current.addEventListener("clipboard", (event) => {
      if (!live()) return
      const text = (event as CustomEvent<{ text: string }>).detail.text
      setRemoteClipboard(text)
      setClipboard((open) => (open === null ? null : text))
    })
    return () => {
      if (rfb.current === current) {
        rfb.current = null
        channel.current = null
      }
      current.disconnect()
      media.close()
    }
  }, [id, running, attemptKey, media])

  // Applied to every new connection too. The manager hears it before noVNC asks for a size, and
  // once no viewer fits the screen, it gives the desktop the resolution of its settings again.
  React.useEffect(() => {
    fits.current = resize
    if (channel.current) tellFit(channel.current, resize)
    if (rfb.current) rfb.current.resizeSession = resize
  }, [resize, attemptKey])

  // Ctrl+W and the like would close the view while the desktop goes on; the browser asks first.
  React.useEffect(() => (connection === "connected" ? holdUnload() : undefined), [connection])

  React.useEffect(() => {
    const keyboard = keyboardLock()
    const update = () => {
      const on = !!document.fullscreenElement
      setFullscreen(on)
      if (!keyboard) return
      if (on) keyboard.lock().catch(() => {})
      else keyboard.unlock()
    }
    document.addEventListener("fullscreenchange", update)
    return () => {
      document.removeEventListener("fullscreenchange", update)
      keyboard?.unlock()
    }
  }, [])

  const name = item?.spec.name ?? t("nav.desktop")
  const spec = item?.spec

  return (
    <section ref={view} className="flex h-dvh flex-col bg-background">
      <header className="flex h-12 shrink-0 items-center gap-1 border-b bg-sidebar px-2 sm:gap-2 sm:px-3">
        <Button variant="ghost" size="sm" asChild>
          <a href={href({ page: "containers" })}>
            <RiArrowLeftLine data-icon="inline-start" />
            <span className="hidden md:inline">{t("nav.containers")}</span>
            <span className="sr-only md:hidden">{t("desktop.back")}</span>
          </a>
        </Button>
        <Separator
          orientation="vertical"
          className="mx-1 data-vertical:h-4 data-vertical:self-center"
        />
        <h1 className="min-w-0 truncate text-sm font-medium">{name}</h1>
        {running && (
          <span
            className="flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground"
            role="status"
          >
            <span
              aria-hidden
              className={cn(
                "size-2 rounded-full",
                connection === "connected"
                  ? "bg-success"
                  : connection === "connecting"
                    ? "animate-pulse bg-warning"
                    : "bg-muted-foreground"
              )}
            />
            {/* Below 640 px only the dot shows; screen readers still hear the state. */}
            <span className="sr-only sm:not-sr-only">{connectionText(connection)}</span>
          </span>
        )}
        <div className="ml-auto flex items-center gap-0.5">
          {running && (
            <>
              <ToolButton
                label={t("uploads.title")}
                icon={<RiUploadLine />}
                onClick={() => openDialog({ kind: "upload", id })}
              />
              <ToolButton
                label={t("downloads.title")}
                icon={<RiDownloadLine />}
                onClick={() => openDialog({ kind: "download", id })}
              />
              <ToolButton
                label={t("desktop.clipboard")}
                icon={<RiClipboardLine />}
                onClick={() => setClipboard(remoteClipboard)}
              />
              {spec?.audio && (
                <ToolButton
                  label={
                    audio.output
                      ? t("desktop.mute")
                      : audio.outputPending
                        ? t("desktop.startingAudio")
                        : t("spec.audio")
                  }
                  icon={
                    audio.output ? (
                      <RiVolumeUpLine className="text-primary" />
                    ) : (
                      <RiVolumeMuteLine />
                    )
                  }
                  aria-pressed={audio.output}
                  onClick={() => attempt(() => media.toggleOutput())}
                />
              )}
              {spec?.microphone && (
                <ToolButton
                  label={
                    audio.input
                      ? t("desktop.micOff")
                      : audio.inputPending
                        ? t("desktop.micPermission")
                        : t("spec.microphone")
                  }
                  icon={audio.input ? <RiMicLine className="text-primary" /> : <RiMicOffLine />}
                  aria-pressed={audio.input}
                  onClick={() => attempt(() => media.toggleMic())}
                />
              )}
              <ToolButton
                label={t("desktop.resize")}
                hint={t("desktop.resizeHint")}
                icon={<RiAspectRatioLine className={cn(resize && "text-primary")} />}
                aria-pressed={resize}
                className={cn(resize && "bg-muted")}
                onClick={() => {
                  storeResize(!resize)
                  setResize(!resize)
                }}
              />
              <ToolButton
                label={t("desktop.reconnect")}
                icon={<RiRestartLine />}
                onClick={() => setGeneration((value) => value + 1)}
              />
            </>
          )}
          <JobsMenu labelFrom="xl" />
          <ToolButton
            label={fullscreen ? t("desktop.exitFullscreen") : t("desktop.fullscreen")}
            icon={fullscreen ? <RiFullscreenExitLine /> : <RiFullscreenLine />}
            onClick={() =>
              attempt(() =>
                document.fullscreenElement
                  ? document.exitFullscreen()
                  : view.current!.requestFullscreen()
              )
            }
          />
        </div>
      </header>
      <HumanRequestStrip container={id} />
      <div
        {...drop.props}
        className={cn(
          "relative min-h-0 flex-1 bg-[#0a0a0a]",
          drop.over && "outline-3 -outline-offset-3 outline-primary"
        )}
      >
        {running ? (
          <>
            <div ref={screen} className="vnc-screen size-full overflow-hidden" />
            {connection !== "connected" && (
              <ConnectionOverlay
                connection={connection}
                onReconnect={() => setGeneration((value) => value + 1)}
              />
            )}
          </>
        ) : (
          <Empty className="h-full text-white/80">
            <EmptyHeader>
              <EmptyTitle className="text-base">
                {containers.isPending
                  ? t("desktop.loading")
                  : item
                    ? t("desktop.notRunning", { name })
                    : t("containers.gone")}
              </EmptyTitle>
              {item && <EmptyDescription>{t("desktop.startToConnect")}</EmptyDescription>}
            </EmptyHeader>
            <EmptyContent>
              {containers.isPending ? (
                <Spinner />
              ) : item ? (
                <Button onClick={() => attempt(() => containerAction(id, "start"))}>
                  <RiPlayCircleLine data-icon="inline-start" />
                  {t("containers.start")}
                </Button>
              ) : (
                <Button variant="outline" asChild>
                  <a href={href({ page: "containers" })}>{t("desktop.back")}</a>
                </Button>
              )}
            </EmptyContent>
          </Empty>
        )}
      </div>
      <Dialog open={clipboard !== null} onOpenChange={(open) => !open && setClipboard(null)}>
        <DialogContent className="flex max-h-[calc(100dvh-2rem)] flex-col sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("desktop.clipboard")}</DialogTitle>
            <DialogDescription>{t("desktop.clipboardHint")}</DialogDescription>
          </DialogHeader>
          <Textarea
            rows={8}
            aria-label={t("desktop.clipboardText")}
            placeholder={t("desktop.clipboardPlaceholder")}
            value={clipboard ?? ""}
            onChange={(event) => setClipboard(event.target.value)}
            className="max-h-[500px] overflow-y-auto"
          />
          <DialogFooter>
            <Button
              onClick={() => {
                rfb.current?.clipboardPasteFrom(clipboard ?? "")
                setClipboard(null)
              }}
            >
              {t("desktop.send")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  )
}
