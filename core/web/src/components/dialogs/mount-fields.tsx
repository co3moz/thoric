import * as React from "react"
import { RiAddLine, RiCloseLine } from "@remixicon/react"
import { cn } from "cn"

import { ConceptIcon } from "@/components/concept"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { FieldDescription, FieldGroup, FieldLegend, FieldSet } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { SetupVolume } from "@/lib/compose"
import { t } from "@/lib/i18n"
import type { Mount, Volume } from "@/lib/types"
import { tile } from "./form-sections"
import { VolumeDialog } from "./simple-dialogs"
import { nextMountKey, type SpecDraft } from "./spec-fields"

// Volume names never contain NUL, so this cannot collide with one.
const CREATE = "\u0000create"

/**
 * The volumes of a container: those its image setup mounts, which the setup's values choose and
 * this only lists, then the ones mounted here, each with its path and access. `volumes` are the
 * ones a container may mount. A volume created here, from a mount's choice or from **Mount** while
 * there is none, opens over the form and goes to that mount, or to a new one.
 */
export function MountFields({
  draft,
  onChange,
  volumes,
  setup,
}: {
  draft: SpecDraft
  onChange: (draft: SpecDraft) => void
  volumes: Volume[]
  setup: SetupVolume[]
}) {
  const set = (mounts: SpecDraft["mounts"]) => onChange({ ...draft, mounts })
  const setMount = (key: number, patch: Partial<Mount>) =>
    set(draft.mounts.map((mount) => (mount.key === key ? { ...mount, ...patch } : mount)))
  const addMount = (volume: string) =>
    set([...draft.mounts, { key: nextMountKey(), volume, target: "", readonly: false }])
  // The mount a volume being created goes to, or `"new"` for one it adds.
  const [creating, setCreating] = React.useState<number | "new" | null>(null)
  // The mount whose choice asked for a new volume, until that choice has closed.
  const asked = React.useRef<number | null>(null)
  const created = (volume: string) => {
    if (creating === "new") addMount(volume)
    else if (creating !== null) setMount(creating, { volume })
    setCreating(null)
  }

  return (
    <FieldGroup>
      {setup.length > 0 && (
        <FieldSet className="gap-2">
          <FieldLegend variant="label" className="mb-1">
            {t("spec.setupVolumes")}
          </FieldLegend>
          {setup.map((mount) => (
            <div
              key={mount.target}
              className={cn("flex min-h-10 items-center gap-2.5 px-3 py-2 text-sm", tile)}
            >
              <ConceptIcon concept="volume" />
              <span className="min-w-0 truncate font-mono text-xs">{mount.target}</span>
              <span className="ml-auto min-w-0 truncate text-right text-xs text-muted-foreground">
                {mount.volume ? (
                  <span className="font-mono">{mount.volume}</span>
                ) : (
                  t("spec.setupVolumes.new")
                )}
                {mount.step && ` · ${mount.step}`}
              </span>
            </div>
          ))}
        </FieldSet>
      )}
      <FieldSet className="gap-3">
        <div className="flex items-center justify-between">
          <FieldLegend variant="label" className="mb-0">
            {t("spec.mounts")}
          </FieldLegend>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => (volumes.length ? addMount(volumes[0].Name) : setCreating("new"))}
          >
            <RiAddLine data-icon="inline-start" />
            {t("spec.mount")}
          </Button>
        </div>
        {draft.mounts.map((mount) => {
          const names = volumes.map((volume) => volume.Name)
          // A container's existing mount stays visible, so it can be removed or changed.
          if (!names.includes(mount.volume)) names.push(mount.volume)
          return (
            <div
              key={mount.key}
              className="grid grid-cols-[1fr_auto_auto] items-center gap-2 sm:grid-cols-[12rem_1fr_auto_auto]"
            >
              <Select
                value={mount.volume}
                onValueChange={(volume) => {
                  if (volume === CREATE) asked.current = mount.key
                  else setMount(mount.key, { volume })
                }}
              >
                <SelectTrigger aria-label={t("spec.volume")} className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent
                  // The dialog opens once the choice has closed and focused its trigger, which
                  // the dialog gives the focus back to. Opened with the choice, it lost its name
                  // field's focus to that trigger.
                  onCloseAutoFocus={() => {
                    if (asked.current === null) return
                    setCreating(asked.current)
                    asked.current = null
                  }}
                >
                  <SelectGroup>
                    {names.map((name) => (
                      <SelectItem key={name} value={name}>
                        <ConceptIcon concept="volume" />
                        {name}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                  <SelectSeparator />
                  <SelectGroup>
                    <SelectItem value={CREATE}>
                      <RiAddLine />
                      {t("spec.createVolume")}
                    </SelectItem>
                  </SelectGroup>
                </SelectContent>
              </Select>
              <Input
                aria-label={t("spec.mountPath")}
                placeholder="/workspace/project"
                required
                value={mount.target}
                onChange={(event) => setMount(mount.key, { target: event.target.value })}
                className="col-span-3 row-start-2 font-mono sm:col-span-1 sm:row-start-auto"
              />
              <Label className="flex items-center gap-2 text-xs font-normal">
                <Checkbox
                  checked={mount.readonly}
                  onCheckedChange={(value) => setMount(mount.key, { readonly: !!value })}
                />
                {t("spec.readOnly")}
              </Label>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={t("spec.removeMount")}
                onClick={() => set(draft.mounts.filter((item) => item.key !== mount.key))}
              >
                <RiCloseLine />
              </Button>
            </div>
          )
        })}
        <FieldDescription className="text-xs">{t("spec.mountsHint")}</FieldDescription>
      </FieldSet>
      <VolumeDialog
        open={creating !== null}
        onOpenChange={(open) => !open && setCreating(null)}
        onCreated={created}
      />
    </FieldGroup>
  )
}
