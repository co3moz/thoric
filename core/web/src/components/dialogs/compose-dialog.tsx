import * as React from "react"
import { RiAddLine, RiAlertLine, RiDeleteBinLine } from "@remixicon/react"
import { useQuery } from "@tanstack/react-query"
import { toast } from "sonner"

import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import { Switch } from "@/components/ui/switch"
import { api } from "@/lib/api"
import { closeDialog, useDraft } from "@/lib/dialogs"
import { refreshLists, useImages } from "@/lib/queries"
import {
  newParameter,
  composeBody,
  composeDraft,
  composeImage,
  imageSteps,
  stepTitle,
  type ParameterDraft,
  type ComposeDraft,
  type StepDraft,
} from "@/lib/compose"
import { stepVariables } from "@/lib/script-completions"
import type { Compose } from "@/lib/types"
import { useSubmit } from "./container-dialogs"
import type { DialogProps } from "./side-drawer"
import { formatList, plural, t } from "@/lib/i18n"
import { NumberedStep, NumberedSteps } from "./numbered-steps"
import { ParameterEditor } from "./parameter-editor"
import { ScriptField } from "./script-field"
import { Footer } from "./simple-dialogs"

/**
 * The compose editor shared by Build image and Edit compose. A compose of several steps, such as
 * one merged from templates, numbers its steps in the order they run, each open.
 */
export function ComposeFields({
  draft,
  onChange,
}: {
  draft: ComposeDraft
  onChange: (draft: ComposeDraft) => void
}) {
  const setStep = (key: number, next: StepDraft) =>
    onChange({ steps: draft.steps.map((step) => (step.key === key ? next : step)) })
  if (draft.steps.length === 1) {
    const [step] = draft.steps
    return <StepFields draft={step} onChange={(next) => setStep(step.key, next)} />
  }
  return (
    <NumberedSteps>
      {draft.steps.map((step, index) => (
        <NumberedStep
          key={step.key}
          number={index + 1}
          last={index === draft.steps.length - 1}
          heading={
            <>
              <span className="min-w-0 truncate font-medium">{stepTitle(step)}</span>
              <span className="text-xs text-muted-foreground">
                {plural(step.parameters.length, {
                  one: "compose.parameterCount.one",
                  other: "compose.parameterCount.other",
                })}
              </span>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="ml-auto text-destructive"
                onClick={() =>
                  onChange({ steps: draft.steps.filter((other) => other.key !== step.key) })
                }
              >
                <RiDeleteBinLine data-icon="inline-start" />
                {t("compose.removeStep", { name: stepTitle(step) })}
              </Button>
            </>
          }
        >
          <StepFields draft={step} onChange={(next) => setStep(step.key, next)} />
        </NumberedStep>
      ))}
    </NumberedSteps>
  )
}

/** One step's parameters, script and how it runs. */
function StepFields({
  draft,
  onChange,
}: {
  draft: StepDraft
  onChange: (draft: StepDraft) => void
}) {
  const id = React.useId()
  const set = (patch: Partial<StepDraft>) => onChange({ ...draft, ...patch })
  const setParameter = (key: number, next: ParameterDraft) =>
    set({
      parameters: draft.parameters.map((parameter) => (parameter.key === key ? next : parameter)),
    })

  return (
    <FieldGroup>
      <FieldSet>
        <div className="flex items-center justify-between">
          <FieldLegend variant="label" className="mb-0">
            {t("compose.parameters")}
          </FieldLegend>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={draft.parameters.length >= 32}
            onClick={() => set({ parameters: [...draft.parameters, newParameter()] })}
          >
            <RiAddLine data-icon="inline-start" />
            {t("compose.parameter")}
          </Button>
        </div>
        <FieldDescription className="text-xs">{t("compose.parametersHint")}</FieldDescription>
        {draft.parameters.map((parameter) => (
          <ParameterEditor
            key={parameter.key}
            parameter={parameter}
            onChange={(next) => setParameter(parameter.key, next)}
            onRemove={() =>
              set({ parameters: draft.parameters.filter((item) => item.key !== parameter.key) })
            }
          />
        ))}
      </FieldSet>
      <Field>
        <FieldLabel htmlFor={`${id}-script`}>{t("compose.script")}</FieldLabel>
        <ScriptField
          id={`${id}-script`}
          label={
            draft.name || draft.id
              ? `${t("compose.script")} · ${stepTitle(draft)}`
              : t("compose.script")
          }
          value={draft.script}
          onChange={(script) => set({ script })}
          required
          hint={t("compose.scriptHint")}
          completions={{ place: "setup", variables: stepVariables(draft.parameters) }}
        />
        <FieldDescription className="text-xs">{t("compose.scriptHint")}</FieldDescription>
      </Field>
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <Field orientation="horizontal">
          <Switch
            id={`${id}-root`}
            checked={draft.root}
            onCheckedChange={(root) => set({ root })}
          />
          <FieldLabel htmlFor={`${id}-root`}>{t("compose.root")}</FieldLabel>
        </Field>
        <Field>
          <FieldLabel htmlFor={`${id}-timeout`}>{t("compose.timeout")}</FieldLabel>
          <Input
            id={`${id}-timeout`}
            type="number"
            min={1}
            max={7200}
            required
            value={draft.timeout_seconds}
            onChange={(event) => set({ timeout_seconds: event.target.value })}
          />
        </Field>
      </div>
    </FieldGroup>
  )
}

/**
 * Gives a saved image a compose, changes it or removes it. An image built on a saved image
 * before builds kept their base's compose is offered the steps it lacks.
 */
export function ComposeDialog({ id, ...props }: DialogProps & { id: string }) {
  const images = useImages().data
  const image = images?.find((record) => record.id === id)
  const exists = !!image?.compose
  const base = image?.base_compose_missing
    ? composeImage(images ?? [], image.parent_image_id ?? undefined)
    : undefined
  // The list leaves the script out; the compose route has all of it.
  const saved = useQuery({
    queryKey: ["images", id, "compose"],
    queryFn: () => api<Compose>(`images/${id}/compose`),
    enabled: exists,
    staleTime: 0,
  })
  const [fresh] = React.useState(composeDraft)
  const [draft, setDraft] = React.useState<ComposeDraft>()
  useDraft(draft)
  const { busy, error, submit } = useSubmit()
  const current = draft ?? (exists ? saved.data && composeDraft(saved.data) : fresh)
  const save = async (remove: boolean) => {
    if (remove) await api(`images/${id}/compose`, undefined, "DELETE")
    else if (current) await api(`images/${id}/compose`, composeBody(current), "PUT")
    else return
    closeDialog()
    toast.success(remove ? t("compose.removed") : t("compose.saved", { name: image?.name ?? "" }))
    void refreshLists("images")
  }
  // Nothing is built again: the manager puts the base's steps before the image's own.
  const addBase = async () => {
    if (!base) return
    await api(`images/${id}/compose/base`, {})
    closeDialog()
    toast.success(t("compose.baseAdded", { base: base.name, name: image?.name ?? "" }))
    void refreshLists("images")
  }

  return (
    <Dialog {...props}>
      <DialogContent className="sm:max-w-2xl">
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            // Remove submits the form too, without the browser checking its fields.
            const submitter = (event.nativeEvent as SubmitEvent).submitter
            return submit(() => save(submitter?.dataset.action === "remove"))(event)
          }}
        >
          <DialogHeader>
            <DialogTitle>{exists ? t("compose.edit") : t("compose.add")}</DialogTitle>
            <DialogDescription>
              {image ? t("compose.descriptionOf", { name: image.name }) : t("compose.description")}
            </DialogDescription>
          </DialogHeader>
          <div className="-mx-4 flex max-h-[min(36rem,60vh)] flex-col gap-5 overflow-y-auto px-4 py-1">
            {base && image && (
              <Alert>
                <RiAlertLine className="text-warning" />
                <AlertDescription className="flex flex-col items-start gap-3">
                  <p>
                    {t("compose.baseMissing", { name: image.name, base: base.name })}
                    {imageSteps(base).length > 1 &&
                      ` ${t("compose.baseSteps", { steps: formatList(imageSteps(base)) })}`}
                  </p>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    // Edits made here would be lost, so they are saved or dropped first.
                    disabled={busy || !!draft}
                    onClick={(event) => submit(addBase)(event)}
                  >
                    <RiAddLine data-icon="inline-start" />
                    {t("compose.addBase", { base: base.name })}
                  </Button>
                </AlertDescription>
              </Alert>
            )}
            {current ? (
              <>
                <ComposeFields draft={current} onChange={setDraft} />
                {exists && (
                  <FieldDescription className="text-xs">{t("compose.pinned")}</FieldDescription>
                )}
              </>
            ) : saved.isError ? (
              <FieldError>{saved.error.message}</FieldError>
            ) : (
              <Skeleton className="h-72 rounded-xl" />
            )}
          </div>
          <FieldError>{error}</FieldError>
          <Footer busy={busy} label={t("compose.save")} disabled={!current}>
            {exists && (
              <Button
                type="submit"
                formNoValidate
                data-action="remove"
                variant="outline"
                className="text-destructive sm:mr-auto"
                disabled={busy}
              >
                {t("compose.remove")}
              </Button>
            )}
          </Footer>
        </form>
      </DialogContent>
    </Dialog>
  )
}
