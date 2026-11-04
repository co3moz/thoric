import { RiAddLine, RiCloseLine } from "@remixicon/react"
import { cn } from "cn"
import { toast } from "sonner"

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
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { SetupVolume } from "@/lib/compose"
import { t } from "@/lib/i18n"
import type { Mount, Volume } from "@/lib/types"
import { tile } from "./form-sections"
import { nextMountKey, type SpecDraft } from "./spec-fields"

/**
 * The volumes of a container: those its image setup mounts, which the setup's values choose and
 * this only lists, then the ones mounted here, each with its path and access. `volumes` are the
 * ones a container may mount.
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
            onClick={() => {
              if (!volumes.length) {
                toast.error(t("spec.volumeFirst"))
                return
              }
              set([
                ...draft.mounts,
                { key: nextMountKey(), volume: volumes[0].Name, target: "", readonly: false },
              ])
            }}
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
                onValueChange={(volume) => setMount(mount.key, { volume })}
              >
                <SelectTrigger aria-label={t("spec.volume")} className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {names.map((name) => (
                      <SelectItem key={name} value={name}>
                        <ConceptIcon concept="volume" />
                        {name}
                      </SelectItem>
                    ))}
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
    </FieldGroup>
  )
}
