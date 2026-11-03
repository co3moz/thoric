import * as React from "react"
import { RiAlertLine } from "@remixicon/react"
import { toast } from "sonner"

import { ImageChoiceItem } from "@/components/image-icon"
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/ui/field"
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
import { closeDialog, useDraft } from "@/lib/dialogs"
import { imageChoices, initialImage, restartNotice } from "@/lib/format"
import { knownGroups } from "@/lib/groups"
import { formatList, plural, t } from "@/lib/i18n"
import { imageServers } from "@/lib/mcp"
import { useContainers, useImages, useSettings, useVolumes } from "@/lib/queries"
import { composeEntries, composeFor, composeSteps, parameterBody, stepTitle } from "@/lib/compose"
import { lifecycleBody, lifecycleDraft, type LifecycleDraft } from "@/lib/lifecycle"
import type { ImageItem, SettingsPreview } from "@/lib/types"
import type { DialogProps } from "./side-drawer"
import { identityBody, identityDraft, IdentityFields, type IdentityDraft } from "./identity-fields"
import { LifecycleFields } from "./lifecycle-fields"
import { ParameterField } from "./parameter-field"
import { specBody, specDraft, SpecFields, type SpecDraft } from "./spec-fields"

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

const scrollBody = "-mx-4 flex max-h-[min(36rem,70vh)] flex-col gap-5 overflow-y-auto px-4 py-1"

/**
 * The values an image's compose asks for, filled in with its defaults. A compose of several
 * steps, such as one merged from templates, asks for each step's values in a section of its own,
 * since two steps may ask for the same name.
 */
function ComposeValues({
  compose,
  values,
  onChange,
}: {
  compose: NonNullable<ImageItem["compose"]>
  values: Record<string, string>
  onChange: (values: Record<string, string>) => void
}) {
  const steps = composeSteps(compose)
  const entries = composeEntries(compose)
  if (!entries.length) return <FieldDescription>{t("compose.runsScript")}</FieldDescription>
  const fields = (step: (typeof steps)[number]) => (
    <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
      {entries
        .filter((entry) => entry.step === step)
        .map(({ parameter, key }) => (
          <ParameterField
            key={`${key}-${parameter.type}`}
            parameter={parameter}
            value={values[key] ?? parameter.default}
            onChange={(value) => onChange({ ...values, [key]: value })}
            hints
            createVolumes
          />
        ))}
    </div>
  )
  return (
    <FieldSet>
      <FieldLegend variant="label">{t("compose.name")}</FieldLegend>
      <FieldDescription className="text-xs">
        {steps.length > 1 ? t("compose.valuesHintSteps") : t("compose.valuesHint")}
      </FieldDescription>
      {steps.length > 1 ? (
        // Every section starts open, so no required value is hidden.
        <Accordion
          type="multiple"
          defaultValue={steps.map((_, index) => String(index))}
          className="rounded-2xl border px-4"
        >
          {steps.map((step, index) => (
            <AccordionItem key={index} value={String(index)}>
              <AccordionTrigger>
                <span className="flex min-w-0 flex-wrap items-center gap-2">
                  <span className="truncate">{stepTitle(step)}</span>
                  <span className="text-xs font-normal text-muted-foreground">
                    {plural(step.parameters.length, {
                      one: "compose.parameterCount.one",
                      other: "compose.parameterCount.other",
                    })}
                  </span>
                </span>
              </AccordionTrigger>
              <AccordionContent className="px-1">
                {step.parameters.length ? (
                  fields(step)
                ) : (
                  <FieldDescription>{t("compose.stepRunsScript")}</FieldDescription>
                )}
              </AccordionContent>
            </AccordionItem>
          ))}
        </Accordion>
      ) : (
        fields(steps[0])
      )}
    </FieldSet>
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
  const compose = composeFor(images, chosen)
  // Values typed for one image are kept while another is chosen, and sent only for its own fields.
  const [values, setValues] = React.useState<Record<string, string>>({})
  const servers = imageServers(images, chosen)
  const [endpoint, setEndpoint] = React.useState(true)
  useDraft({ name, selected, draft, lifecycle, values, endpoint })
  const { busy, error, submit } = useSubmit()
  const id = React.useId()

  return (
    <Dialog {...props}>
      <DialogContent className="sm:max-w-2xl">
        <form
          className="flex flex-col gap-4"
          onSubmit={submit(() =>
            runJob("containers", {
              name,
              image: chosen,
              ...specBody(draft),
              ...lifecycleBody(lifecycle),
              ...(compose ? { parameters: parameterBody(compose, values) } : {}),
              mcp_endpoint: endpoint,
            })
          )}
        >
          <DialogHeader>
            <DialogTitle>{t("containers.new")}</DialogTitle>
            <DialogDescription>{t("create.description")}</DialogDescription>
          </DialogHeader>
          <div className={scrollBody}>
            <FieldGroup>
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
              {compose && <ComposeValues compose={compose} values={values} onChange={setValues} />}
              <Field orientation="horizontal" className="items-start">
                <Checkbox
                  id={`${id}-endpoint`}
                  checked={endpoint}
                  onCheckedChange={(checked) => setEndpoint(checked === true)}
                />
                <FieldContent>
                  <FieldLabel htmlFor={`${id}-endpoint`}>{t("create.mcpEndpoint")}</FieldLabel>
                  <FieldDescription className="text-xs">
                    {t("create.mcpEndpointHint")}
                  </FieldDescription>
                </FieldContent>
              </Field>
            </FieldGroup>
            <SpecFields
              draft={draft}
              onChange={setDraft}
              settings={settings}
              volumes={volumes}
              groups={groups}
            />
            <LifecycleFields draft={lifecycle} onChange={setLifecycle} />
          </div>
          {/* Outside the scrolling fields, so a long form cannot hide the outcome. */}
          <FieldError>{error}</FieldError>
          <DialogFooter>
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
  useDraft({ name, draft, identity, lifecycle })
  // A restart warning is confirmed for one set of values; editing the form asks again.
  const [notice, setNotice] = React.useState("")
  const { busy, error, submit } = useSubmit()
  const formId = React.useId()
  const running = item?.state === "running"

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
  return (
    <Dialog {...props}>
      <DialogContent className="sm:max-w-2xl">
        <form className="flex flex-col gap-4" onSubmit={submit(save)}>
          <DialogHeader>
            <DialogTitle>{t("settings.title")}</DialogTitle>
            <DialogDescription>{t("settings.description")}</DialogDescription>
          </DialogHeader>
          <div className={scrollBody}>
            <Field>
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
            <SpecFields
              draft={draft}
              onChange={setDraft}
              settings={settings}
              volumes={volumes}
              spec={spec}
              groups={knownGroups(containers.filter((other) => other.id !== id))}
              vncPrivate={item?.vnc_private}
              // These settings are what this dialog is for, so they start in view.
              advancedOpen
            />
            <LifecycleFields draft={lifecycle} onChange={setLifecycle} start={lifecycleStart} />
            <IdentityFields draft={identity} onChange={setIdentity} spec={spec} />
          </div>
          {/* Outside the scrolling fields, next to the button whose label it changes. */}
          {notice && (
            <Alert>
              <RiAlertLine className="text-warning" />
              <AlertDescription>{notice}</AlertDescription>
            </Alert>
          )}
          <FieldError>{error}</FieldError>
          <DialogFooter>
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
        </form>
      </DialogContent>
    </Dialog>
  )
}
