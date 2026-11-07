import * as React from "react"

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import {
  confirmDiscard,
  DialogScope,
  dismissDialog,
  keepEditing,
  useDialogState,
  type Dialog,
  type Unsaved,
} from "@/lib/dialogs"
import { t } from "@/lib/i18n"
import { ActivityDrawer } from "./activity-drawer"
import { ContainerDrawer } from "./container-drawer"
import { CreateContainerDialog, SettingsDialog } from "./container-dialogs"
import { DuplicateDialog } from "./duplicate-dialog"
import {
  MakeScalableDialog,
  NewInstanceDialog,
  ScaleSetDeleteDialog,
  ScaleSetDialog,
} from "./scale-dialogs"
import { GroupConnectDialog } from "./group-dialog"
import { JobDrawer } from "./job-drawer"
import { LogDialog } from "./log-dialog"
import { ProcessesDrawer } from "./processes-drawer"
import { ImportImageDialog } from "./import-dialog"
import {
  DeleteDialog,
  ImageDeleteDialog,
  ResetDialog,
  SnapshotDialog,
  VolumeCopyDialog,
  VolumeDeleteDialog,
  VolumeDialog,
} from "./simple-dialogs"
import { BuildImageDialog } from "./build-dialog"
import { ComposeDialog } from "./compose-dialog"
import { ConnectDialog } from "./connect-dialog"
import { EndpointDialog } from "./endpoint-dialog"
import { EndpointConfirmDialog, ServerRemoveDialog, TokenDialog, ToolsDialog } from "./mcp-dialogs"
import { ServerDialog } from "./server-dialog"
import { IconDialog } from "./icon-dialog"
import { ImageDrawer } from "./image-drawer"
import { UploadDialog } from "./upload-dialog"
import { UpdateDialog } from "./update-dialog"
import { RestoreDialog } from "./restore-dialog"
import { DownloadDialog } from "./download-dialog"
import { VolumeBrowser } from "./volume-browser"
import { VolumeImportDialog } from "./volume-import-dialog"

/**
 * Renders the one open dialog, and the question asked before it closes with something unsaved;
 * see lib/dialogs.ts.
 */
export function DialogHost() {
  const { dialog, open, seq, asking } = useDialogState()
  return (
    <>
      {/* A new `seq` remounts the dialog, so every opening starts with a fresh form. */}
      {dialog && (
        <DialogScope key={seq} value={seq}>
          {render(dialog, open)}
        </DialogScope>
      )}
      <DiscardQuestion reason={asking?.reason ?? null} />
    </>
  )
}

const questions = {
  changes: {
    title: "unsaved.changes.title",
    description: "unsaved.changes.description",
    keep: "unsaved.changes.keep",
    discard: "unsaved.changes.discard",
  },
  upload: {
    title: "unsaved.upload.title",
    description: "unsaved.upload.description",
    keep: "unsaved.upload.keep",
    discard: "unsaved.upload.discard",
  },
} as const

/** Asks before a form's changes or running uploads are lost, over the dialog that holds them. */
function DiscardQuestion({ reason }: { reason: Unsaved | null }) {
  // The question keeps its words while it closes.
  const [shown, setShown] = React.useState<Unsaved>("changes")
  if (reason && reason !== shown) setShown(reason)
  const keys = questions[reason ?? shown]
  return (
    <AlertDialog open={!!reason} onOpenChange={(next) => !next && keepEditing()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t(keys.title)}</AlertDialogTitle>
          <AlertDialogDescription>{t(keys.description)}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t(keys.keep)}</AlertDialogCancel>
          <Button variant="destructive" onClick={confirmDiscard}>
            {t(keys.discard)}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

function render(dialog: Dialog, open: boolean) {
  // Escape, a click outside, a close button and Cancel ask first when the dialog holds something.
  const props = { open, onOpenChange: (next: boolean) => !next && dismissDialog() }
  switch (dialog.kind) {
    case "create":
      return <CreateContainerDialog {...props} image={dialog.image} />
    case "details":
      return <ContainerDrawer {...props} id={dialog.id} />
    case "settings":
      return <SettingsDialog {...props} id={dialog.id} />
    case "snapshot":
      return <SnapshotDialog {...props} id={dialog.id} />
    case "duplicate":
      return <DuplicateDialog {...props} id={dialog.id} />
    case "make-scalable":
      return <MakeScalableDialog {...props} id={dialog.id} />
    case "scale-set":
      return <ScaleSetDialog {...props} name={dialog.name} />
    case "scale-instance":
      return <NewInstanceDialog {...props} name={dialog.name} />
    case "scale-set-delete":
      return <ScaleSetDeleteDialog {...props} name={dialog.name} />
    case "delete":
      return <DeleteDialog {...props} id={dialog.id} />
    case "reset":
      return <ResetDialog {...props} id={dialog.id} />
    case "upload":
      return <UploadDialog {...props} id={dialog.id} files={dialog.files} />
    case "download":
      return <DownloadDialog {...props} id={dialog.id} />
    case "build":
      return <BuildImageDialog {...props} config={dialog.config} simple={dialog.simple} />
    case "compose":
      return <ComposeDialog {...props} id={dialog.id} />
    case "icon":
      return <IconDialog {...props} id={dialog.id} />
    case "image":
      return <ImageDrawer {...props} id={dialog.id} />
    case "image-delete":
      return <ImageDeleteDialog {...props} id={dialog.id} />
    case "import":
      return <ImportImageDialog {...props} />
    case "volume":
      return <VolumeDialog {...props} />
    case "volume-files":
      return <VolumeBrowser {...props} name={dialog.name} />
    case "volume-import":
      return <VolumeImportDialog {...props} />
    case "volume-copy":
      return <VolumeCopyDialog {...props} name={dialog.name} />
    case "group-connect":
      return <GroupConnectDialog {...props} group={dialog.group} />
    case "volume-delete":
      return <VolumeDeleteDialog {...props} name={dialog.name} />
    case "job":
      return <JobDrawer {...props} id={dialog.id} />
    case "processes":
      return <ProcessesDrawer {...props} id={dialog.id} />
    case "log":
      return <LogDialog {...props} title={dialog.title} text={dialog.text} status={dialog.status} />
    case "mcp-endpoint":
      return <EndpointDialog {...props} name={dialog.name} />
    case "mcp-token":
      return <TokenDialog {...props} token={dialog.token} />
    case "mcp-endpoint-confirm":
      return <EndpointConfirmDialog {...props} name={dialog.name} action={dialog.action} />
    case "mcp-tools":
      return <ToolsDialog {...props} name={dialog.name} />
    case "mcp-connect":
      return <ConnectDialog {...props} name={dialog.name} />
    case "mcp-server":
      return <ServerDialog {...props} container={dialog.container} />
    case "mcp-server-remove":
      return <ServerRemoveDialog {...props} container={dialog.container} name={dialog.name} />
    case "activity":
      return <ActivityDrawer {...props} endpoint={dialog.endpoint} />
    case "update":
      return <UpdateDialog {...props} />
    case "restore":
      return (
        <RestoreDialog
          {...props}
          file={dialog.file}
          manifest={dialog.manifest}
          review={dialog.review}
          limit={dialog.limit}
        />
      )
  }
}
