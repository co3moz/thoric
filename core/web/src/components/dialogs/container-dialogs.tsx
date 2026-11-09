import * as React from "react"
import {
  RiAlertLine,
  RiAspectRatioLine,
  RiBox3Line,
  RiDatabase2Line,
  RiFingerprintLine,
  RiListCheck3,
  RiPlugLine,
  RiRouterLine,
  RiStackLine,
  RiTimerLine,
} from "@remixicon/react"
import { toast } from "sonner"

import { ImageChoiceItem } from "@/components/image-icon"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Spinner } from "@/components/ui/spinner"
import { runJob } from "@/lib/actions"
import { api } from "@/lib/api"
import {
  composeImage,
  inheritedSteps,
  missingValues,
  parameterBody,
  setupVolumes,
  type SetupVolume,
} from "@/lib/compose"
import { closeDialog, useDraft } from "@/lib/dialogs"
import { imageChoices, initialImage, restartNotice } from "@/lib/format"
import { type KnownGroup, knownGroups } from "@/lib/groups"
import { formatList, t } from "@/lib/i18n"
import {
  lifecycleBody,
  lifecycleDraft,
  lifecycleSummary,
  type LifecycleDraft,
} from "@/lib/lifecycle"
import { imageServers } from "@/lib/mcp"
import { useContainers, useImages, useSettings, useVolumes } from "@/lib/queries"
import {
  scalingBody,
  scalingDraft,
  scalingError,
  scalingTargets,
  type ScalingDraft,
} from "@/lib/scale"
import type { McpProfile, Settings, SettingsPreview, Volume } from "@/lib/types"
import { mountableVolumes } from "@/lib/volume-files"
import { FormSections, sectionedDialog, type FormSection } from "./form-sections"
import { identityBody, identityDraft, IdentityFields, type IdentityDraft } from "./identity-fields"
import { LifecycleFields } from "./lifecycle-fields"
import { MountFields } from "./mount-fields"
import { NetworkFields } from "./network-fields"
import { ProfileSelect } from "./profile-select"
import { ScalingFields } from "./scaling-fields"
import { ScreenFields } from "./screen-fields"
import { SetupFields } from "./setup-fields"
import type { DialogProps } from "./side-drawer"
import {
  FeatureFields,
  specBody,
  specDraft,
  specSummary,
  SwitchTile,
  type SpecDraft,
} from "./spec-fields"

/** A dialog whose form submits once at a time and shows the manager's error in place. */
export function useSubmit() {
  const [busy, setBusy] = React.useState(false)
  const [error, setError] = React.useState("")
  const submit = (action: () => Promise<unknown>) => async (event: React.FormEvent) => {
    event.preventDefault()
    setBusy(true)
    setError("")
    try {
      await action()
    } catch (failure) {
      setError((failure as Error).message)
    } finally {
      setBusy(false)
    }
  }
  return { busy, error, setError, submit }
}

/**
 * The sections the create and settings dialogs share, after their own first ones. `setup` are the
 * volumes the image setup mounts, and `groups` the groups other containers list.
 */
function optionSections({
  draft,
  setDraft,
  lifecycle,
  setLifecycle,
  lifecycleStart,
  settings,
  volumes,
  setup,
  groups,
  vncPrivate,
}: {
  draft: SpecDraft
  setDraft: (draft: SpecDraft) => void
  lifecycle: LifecycleDraft
  setLifecycle: (draft: LifecycleDraft) => void
  lifecycleStart?: LifecycleDraft
  settings?: Settings
  volumes: Volume[]
  setup: SetupVolume[]
  groups: KnownGroup[]
  vncPrivate?: boolean | null
}): FormSection[] {
  const group = t("sections.options")
  return [
    {
      id: "screen",
      label: t("sections.screen"),
      icon: RiAspectRatioLine,
      group,
      content: <ScreenFields draft={draft} onChange={setDraft} />,
    },
    {
      id: "network",
      label: t("sections.network"),
      icon: RiRouterLine,
      group,
      content: (
        <NetworkFields
          draft={draft}
          onChange={setDraft}
          settings={settings}
          groups={groups}
          vncPrivate={vncPrivate}
          basics="basics"
        />
      ),
    },
    {
      id: "volumes",
      label: t("nav.volumes"),
      icon: RiDatabase2Line,
      group,
      content: (
        <MountFields
          draft={draft}
          onChange={setDraft}
          volumes={mountableVolumes(volumes)}
          setup={setup}
        />
      ),
    },
    {
      id: "lifecycle",
      label: t("lifecycle.title"),
      icon: RiTimerLine,
      group,
      content: <LifecycleFields draft={lifecycle} onChange={setLifecycle} start={lifecycleStart} />,
    },
  ]
}

/** The form's choices in one line beside its buttons, so they are known from any section. */
function FormSummary({ parts }: { parts: (string | undefined)[] }) {
  return (
    <p className="hidden min-w-0 flex-1 truncate text-xs text-muted-foreground sm:block">
      {parts.filter(Boolean).join(" · ")}
    </p>
  )
}

export function CreateContainerDialog({ image, ...props }: DialogProps & { image?: string }) {
  const settings = useSettings().data
  const images = useImages().data ?? []
  const volumes = useVolumes().data ?? []
  const groups = knownGroups(useContainers().data ?? [])
  const choices = imageChoices(images, settings?.base_image, image)
  const [name, setName] = React.useState("")
  const [selected, setSelected] = React.useState<string | undefined>()
  const [draft, setDraft] = React.useState<SpecDraft>(() => specDraft({}, settings))
  const [lifecycle, setLifecycle] = React.useState<LifecycleDraft>(() =>
    lifecycleDraft({}, settings)
  )
  const chosen = selected ?? initialImage(choices, image ?? settings?.base_image)
  const record = composeImage(images, chosen)
  const compose = record?.compose ?? undefined
  // Values typed for one image are kept while another is chosen, and sent only for its own fields.
  const [values, setValues] = React.useState<Record<string, string>>({})
  const servers = imageServers(images, chosen)
  const [endpoint, setEndpoint] = React.useState(true)
  const [profile, setProfile] = React.useState<McpProfile>("full")
  const [scaling, setScaling] = React.useState<ScalingDraft>(() => scalingDraft())
  const [section, setSection] = React.useState("basics")
  useDraft({ name, selected, draft, lifecycle, values, endpoint, profile, scaling })
  const { busy, error, submit } = useSubmit()
  const id = React.useId()
  const missing = compose ? missingValues(compose, values).reduce((sum, n) => sum + n, 0) : 0
  const [now] = React.useState(() => Math.floor(Date.now() / 1000))

  const container = t("sections.container")
  const sections: FormSection[] = [
    {
      id: "basics",
      label: t("sections.basics"),
      icon: RiBox3Line,
      group: container,
      content: (
        <FieldGroup>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Field>
              <FieldLabel htmlFor={`${id}-name`}>{t("table.name")}</FieldLabel>
              <Input
                id={`${id}-name`}
                required
                maxLength={100}
                placeholder={t("create.namePlaceholder")}
                value={name}
                onChange={(event) => setName(event.target.value)}
                autoFocus
              />
            </Field>
            <Field>
              <FieldLabel htmlFor={`${id}-image`}>{t("create.image")}</FieldLabel>
              <Select value={chosen ?? ""} onValueChange={setSelected}>
                <SelectTrigger id={`${id}-image`} className="w-full">
                  <SelectValue placeholder={t("build.noBase")} />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    {choices.map((choice) => (
                      <ImageChoiceItem key={choice.value} choice={choice} />
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
              {servers.length > 0 && (
                <FieldDescription className="text-xs">
                  {t("images.mcpServers", { names: formatList(servers) })}
                </FieldDescription>
              )}
            </Field>
          </div>
          <FeatureFields draft={draft} onChange={setDraft} settings={settings} />
          <Field>
            <SwitchTile
              icon={RiPlugLine}
              label={t("create.mcpEndpoint")}
              checked={endpoint}
              onCheckedChange={setEndpoint}
              describedBy={`${id}-endpoint-hint`}
            />
            <FieldDescription id={`${id}-endpoint-hint`} className="text-xs">
              {t("create.mcpEndpointHint")}
            </FieldDescription>
          </Field>
          {endpoint && <ProfileSelect id={`${id}-profile`} value={profile} onChange={setProfile} />}
        </FieldGroup>
      ),
    },
    ...(compose
      ? [
          {
            id: "setup",
            label: t("sections.setup"),
            icon: RiListCheck3,
            group: container,
            missing,
            content: (
              <SetupFields
                compose={compose}
                values={values}
                onChange={setValues}
                inherited={inheritedSteps(record, images)}
              />
            ),
          },
        ]
      : []),
    ...optionSections({
      draft,
      setDraft,
      lifecycle,
      setLifecycle,
      settings,
      volumes,
      setup: compose ? setupVolumes(compose, values) : [],
      groups,
    }),
    {
      id: "scaling",
      label: t("scale.title"),
      icon: RiStackLine,
      group: t("sections.options"),
      content: (
        <ScalingFields
          toggle
          draft={scaling}
          onChange={setScaling}
          name={name}
          targets={scalingTargets([
            ...draft.mounts,
            ...(compose ? setupVolumes(compose, values) : []),
          ])}
        />
      ),
    },
  ]

  return (
    <Dialog {...props}>
      <DialogContent className={sectionedDialog}>
        <form
          className="flex min-h-0 flex-1 flex-col"
          onSubmit={submit(async () => {
            // A switch or a set of choices that is required has no field the browser checks.
            if (missing) {
              setSection("setup")
              throw new Error(t("setup.missingError"))
            }
            const scalingProblem = scaling.enabled ? scalingError(scaling) : null
            if (scalingProblem) {
              setSection("scaling")
              throw new Error(t(`scale.error.${scalingProblem}`))
            }
            await runJob("containers", {
              name,
              image: chosen,
              ...specBody(draft),
              ...lifecycleBody(lifecycle),
              ...(compose ? { parameters: parameterBody(compose, values) } : {}),
              mcp_endpoint: endpoint,
              ...(endpoint && profile !== "full" ? { mcp_profile: profile } : {}),
              ...(scaling.enabled ? { scaling: scalingBody(scaling) } : {}),
            })
          })}
        >
          <DialogHeader className="px-6 pt-6 pb-4">
            <DialogTitle>{t("containers.new")}</DialogTitle>
            <DialogDescription>{t("create.description")}</DialogDescription>
          </DialogHeader>
          <FormSections
            sections={sections}
            active={section}
            onActiveChange={setSection}
            label={t("sections.label")}
            className="flex-1"
          />
          {/* Outside the scrolling fields, so a long form cannot hide the outcome. */}
          <div className="flex flex-col gap-3 px-6 pt-3 pb-6">
            <FieldError>{error}</FieldError>
            <DialogFooter className="sm:items-center">
              <FormSummary
                parts={[
                  choices.find((choice) => choice.value === chosen)?.label,
                  ...specSummary(draft),
                  lifecycleSummary(lifecycle, now),
                  scaling.enabled ? t("scale.summary", { max: scaling.max }) : undefined,
                ]}
              />
              <DialogClose asChild>
                <Button type="button" variant="outline">
                  {t("common.cancel")}
                </Button>
              </DialogClose>
              <Button type="submit" disabled={busy || !chosen}>
                {busy && <Spinner data-icon="inline-start" />}
                {t("common.create")}
              </Button>
            </DialogFooter>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

export function SettingsDialog({ id, ...props }: DialogProps & { id: string }) {
  const containers = useContainers().data ?? []
  const item = containers.find((entry) => entry.id === id)
  const settings = useSettings().data
  const volumes = useVolumes().data ?? []
  const spec = item?.spec
  const [name, setName] = React.useState(spec?.name ?? "")
  const [draft, setDraftState] = React.useState<SpecDraft>(() => specDraft(spec ?? {}, settings))
  const [identity, setIdentityState] = React.useState<IdentityDraft>(() =>
    identityDraft(spec ?? {})
  )
  // As the dialog opened: only what the form changes is sent, so a later expiry an agent set
  // meanwhile stays.
  const [lifecycleStart] = React.useState<LifecycleDraft>(() => lifecycleDraft(spec ?? {}))
  const [lifecycle, setLifecycleState] = React.useState(lifecycleStart)
  const [section, setSection] = React.useState("basics")
  useDraft({ name, draft, identity, lifecycle })
  // A restart warning is confirmed for one set of values; editing the form asks again.
  const [notice, setNotice] = React.useState("")
  const { busy, error, submit } = useSubmit()
  const formId = React.useId()
  const running = item?.state === "running"
  const [now] = React.useState(() => Math.floor(Date.now() / 1000))

  const setDraft = (next: SpecDraft) => {
    setNotice("")
    setDraftState(next)
  }
  const setIdentity = (next: IdentityDraft) => {
    setNotice("")
    setIdentityState(next)
  }
  const setLifecycle = (next: LifecycleDraft) => {
    setNotice("")
    setLifecycleState(next)
  }

  const save = async () => {
    const body = {
      name,
      ...specBody(draft),
      ...identityBody(identity),
      ...lifecycleBody(lifecycle, lifecycleStart),
    }
    // The server decides what restarts the container; ask it first and warn once before applying.
    if (!notice) {
      const preview = await api<SettingsPreview>(`containers/${id}/settings`, {
        ...body,
        dry_run: true,
      })
      if (!preview.live.length && !preview.restart.length) {
        closeDialog()
        toast(t("settings.unchanged"))
        return
      }
      if (preview.restart.length) {
        setNotice(restartNotice(preview.restart, running))
        return
      }
    }
    await runJob(`containers/${id}/settings`, body)
  }

  if (!spec) return null
  const container = t("sections.container")
  const sections: FormSection[] = [
    {
      id: "basics",
      label: t("sections.basics"),
      icon: RiBox3Line,
      group: container,
      content: (
        <FieldGroup>
          <Field className="sm:max-w-sm">
            <FieldLabel htmlFor={`${formId}-name`}>{t("table.name")}</FieldLabel>
            <Input
              id={`${formId}-name`}
              required
              maxLength={100}
              value={name}
              onChange={(event) => {
                setNotice("")
                setName(event.target.value)
              }}
            />
          </Field>
          <FeatureFields draft={draft} onChange={setDraft} settings={settings} spec={spec} />
        </FieldGroup>
      ),
    },
    {
      id: "identity",
      label: t("identity.title"),
      icon: RiFingerprintLine,
      group: container,
      content: <IdentityFields draft={identity} onChange={setIdentity} spec={spec} />,
    },
    ...optionSections({
      draft,
      setDraft,
      lifecycle,
      setLifecycle,
      lifecycleStart,
      settings,
      volumes,
      setup: (item?.setup?.mounts ?? []).map(({ volume, target }) => ({ volume, target })),
      groups: knownGroups(containers.filter((other) => other.id !== id)),
      vncPrivate: item?.vnc_private,
    }),
  ]

  return (
    <Dialog {...props}>
      <DialogContent className={sectionedDialog}>
        <form className="flex min-h-0 flex-1 flex-col" onSubmit={submit(save)}>
          <DialogHeader className="px-6 pt-6 pb-4">
            <DialogTitle>{t("settings.title")}</DialogTitle>
            <DialogDescription>{t("settings.description")}</DialogDescription>
          </DialogHeader>
          <FormSections
            sections={sections}
            active={section}
            onActiveChange={setSection}
            label={t("sections.label")}
            className="flex-1"
          />
          {/* Outside the scrolling fields, next to the button whose label it changes. */}
          <div className="flex flex-col gap-3 px-6 pt-3 pb-6">
            {notice && (
              <Alert>
                <RiAlertLine className="text-warning" />
                <AlertDescription>{notice}</AlertDescription>
              </Alert>
            )}
            <FieldError>{error}</FieldError>
            <DialogFooter className="sm:items-center">
              <FormSummary parts={[...specSummary(draft), lifecycleSummary(lifecycle, now)]} />
              <DialogClose asChild>
                <Button type="button" variant="outline">
                  {t("common.cancel")}
                </Button>
              </DialogClose>
              <Button type="submit" disabled={busy}>
                {busy && <Spinner data-icon="inline-start" />}
                {notice && running ? t("settings.saveRestart") : t("common.save")}
              </Button>
            </DialogFooter>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
