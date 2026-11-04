import * as React from "react"
import {
  RiAlertLine,
  RiBox3Line,
  RiClipboardLine,
  RiEqualizerLine,
  RiGlobalLine,
  RiListCheck3,
  RiPlugLine,
  RiTerminalBoxLine,
} from "@remixicon/react"

import { CodeEditor } from "@/components/code-editor"
import { IconPicker, ImageChoiceItem } from "@/components/image-icon"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Dialog,
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
import { Textarea } from "@/components/ui/textarea"
import { runJob } from "@/lib/actions"
import { parseBuildConfig, type ImportedBuild } from "@/lib/build-config"
import {
  composeBody,
  composeDraft,
  composeImage,
  orderSteps,
  type ComposeDraft,
} from "@/lib/compose"
import { useDraft } from "@/lib/dialogs"
import { imageChoices, initialImage } from "@/lib/format"
import { formatList, rich, t } from "@/lib/i18n"
import { useImages, useSettings } from "@/lib/queries"
import { BUILD_SCRIPT } from "@/lib/script-completions"
import {
  optionGroups,
  resolveOptions,
  serversKept,
  type OptionValues,
} from "@/lib/template-options"
import type { DraftPart } from "@/lib/types"
import { BuildBase } from "./build-base"
import { BuildOptions } from "./build-options"
import { BuildParts } from "./build-parts"
import { BuildServers } from "./build-servers"
import { ComposeFields } from "./compose-dialog"
import { useSubmit } from "./container-dialogs"
import { FormSections, sectionedDialog, useShowSection, type FormSection } from "./form-sections"
import { ScriptField } from "./script-field"
import type { DialogProps } from "./side-drawer"
import { Footer } from "./simple-dialogs"
import { SwitchTile } from "./spec-fields"

const DEFAULT_SCRIPT = `#!/usr/bin/env bash
set -euo pipefail
apt-get update
apt-get install -y git build-essential
rm -rf /var/lib/apt/lists/*`

/** The templates a configuration is built from: the one it names, or those merged into it. */
const templatesOf = (config?: ImportedBuild) =>
  config?.template_ids?.length
    ? config.template_ids
    : config?.template_id
      ? [config.template_id]
      : []

/**
 * Where a saved base's compose goes, below the base it comes with, with the way to the section
 * that changes it.
 */
function InheritNote({ name, inherit }: { name: string; inherit: boolean }) {
  const show = useShowSection()
  return (
    <FieldDescription className="text-xs">
      {rich(inherit ? "build.inheritNote" : "build.inheritNoteOff", {
        name,
        change: (
          <Button
            type="button"
            variant="link"
            className="h-auto p-0 text-xs"
            onClick={() => show("compose")}
          >
            {t("build.changeUnderCompose")}
          </Button>
        ),
      })}
    </FieldDescription>
  )
}

/**
 * Builds an image from a script, or from a configuration a template or a paste filled in, in
 * sections: what the image is, a template's options, then its build script, its compose and the
 * desktop MCP servers it declares. Building from the first section builds the rest as configured,
 * which is what a template install does; a `simple` form, opened from a template, says so.
 */
export function BuildImageDialog({
  config,
  simple,
  ...props
}: DialogProps & { config?: ImportedBuild; simple?: boolean }) {
  const settings = useSettings().data
  const images = useImages()
  const choices = imageChoices(images.data ?? [], settings?.base_image)
  const [name, setName] = React.useState(config?.name ?? "")
  const [description, setDescription] = React.useState(config?.description ?? "")
  const [icon, setIcon] = React.useState(config?.icon)
  const [iconError, setIconError] = React.useState<string>()
  const [templateIds, setTemplateIds] = React.useState(() => templatesOf(config))
  const [warnings, setWarnings] = React.useState(config?.warnings ?? [])
  // The base the user chose, and the one a loaded configuration named.
  const [base, setBase] = React.useState<string | undefined>()
  const [wanted, setWanted] = React.useState(config?.base_image)
  const [script, setScript] = React.useState(config?.script ?? DEFAULT_SCRIPT)
  // Templates merged into one build run as parts instead of one script.
  const [parts, setParts] = React.useState(config?.parts ?? null)
  const [internet, setInternet] = React.useState(config?.internet ?? true)
  const [compose, setCompose] = React.useState<ComposeDraft | null>(
    config?.compose ? composeDraft(config.compose) : null
  )
  // A saved base's compose goes before the image's own unless this is turned off.
  const [inherit, setInherit] = React.useState(config?.inherit_compose ?? true)
  // Desktop MCP servers the configuration declares; they are shown, never typed in here.
  const [servers, setServers] = React.useState(config?.mcp ?? [])
  // A template's options, or those of each merged part, and the values chosen for them, which
  // are written into the scripts only when the request is sent.
  const [options, setOptions] = React.useState(config?.options ?? [])
  const [values, setValues] = React.useState<OptionValues>({})
  const [importing, setImporting] = React.useState(false)
  const [section, setSection] = React.useState("basics")
  useDraft({
    name,
    description,
    icon,
    templateIds,
    base,
    wanted,
    script,
    parts,
    internet,
    compose,
    inherit,
    servers,
    options,
    values,
  })
  const groups = optionGroups(options, parts)
  const choose = (key: string, on: boolean) => setValues((current) => ({ ...current, [key]: on }))
  // The servers the image gets: an option that is off leaves its own out.
  const kept = serversKept(servers, groups, values)

  // A part's setup step goes with it: it moves along, and leaves when the part does.
  const moveParts = (next: DraftPart[]) => {
    setParts(next)
    setCompose(
      (current) =>
        current && {
          steps: orderSteps(
            current.steps,
            next.map((part) => part.id)
          ),
        }
    )
  }
  const removePart = (index: number) => {
    const removed = parts?.[index]
    if (!parts || !removed) return
    setParts(parts.filter((_, other) => other !== index))
    setTemplateIds((ids) => ids.filter((id) => id !== removed.id))
    setCompose((current) => {
      const steps = current?.steps.filter((step) => step.id !== removed.id) ?? []
      return steps.length ? { steps } : null
    })
  }
  const usable = choices.some((choice) => choice.value === wanted && !choice.disabled)
  const chosen = base ?? (usable ? wanted : initialImage(choices, settings?.base_image))
  // The base a loaded configuration named when this host cannot build on it.
  const missingBase = base === undefined && !usable && !images.isPending ? wanted : undefined
  // The saved image built on, when new containers from it get a compose.
  const baseImage = composeImage(images.data ?? [], chosen)
  const { busy, error, submit } = useSubmit()
  const field = React.useId()

  const load = (config: ImportedBuild) => {
    if (config.name !== undefined) setName(config.name)
    if (config.description !== undefined) setDescription(config.description)
    if (config.icon !== undefined) setIcon(config.icon)
    if (config.internet !== undefined) setInternet(config.internet)
    setTemplateIds(templatesOf(config))
    setWarnings(config.warnings ?? [])
    setParts(config.parts ?? null)
    if (config.script !== undefined) setScript(config.script)
    setCompose(config.compose ? composeDraft(config.compose) : null)
    setInherit(config.inherit_compose ?? true)
    setServers(config.mcp ?? [])
    setOptions(config.options ?? [])
    setValues({})
    if (config.base_image !== undefined) {
      setWanted(config.base_image)
      setBase(undefined)
    }
    setSection("basics")
    setImporting(false)
  }

  const image = t("sections.image")
  const build = t("sections.build")
  const sections: FormSection[] = [
    {
      id: "basics",
      label: t("sections.basics"),
      icon: RiBox3Line,
      group: image,
      content: (
        <FieldGroup>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Field>
              <FieldLabel htmlFor={`${field}-name`}>{t("table.name")}</FieldLabel>
              <Input
                id={`${field}-name`}
                required
                maxLength={100}
                placeholder={t("build.namePlaceholder")}
                value={name}
                onChange={(event) => setName(event.target.value)}
                autoFocus
              />
            </Field>
            <Field>
              <FieldLabel htmlFor={`${field}-base`}>{t("build.base")}</FieldLabel>
              <Select value={chosen ?? ""} onValueChange={setBase}>
                <SelectTrigger id={`${field}-base`} className="w-full">
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
              {baseImage && <InheritNote name={baseImage.name} inherit={inherit} />}
            </Field>
          </div>
          {missingBase && (
            <Alert>
              <RiAlertLine className="text-warning" />
              <AlertDescription>
                {rich("build.missingBase", {
                  image: <code className="font-mono break-all">{missingBase}</code>,
                })}
              </AlertDescription>
            </Alert>
          )}
          {warnings.map((warning) => (
            <Alert key={warning}>
              <RiAlertLine className="text-warning" />
              <AlertDescription>{warning}</AlertDescription>
            </Alert>
          ))}
          <Field>
            <FieldLabel htmlFor={`${field}-description`}>{t("common.description")}</FieldLabel>
            <Textarea
              id={`${field}-description`}
              rows={2}
              maxLength={4096}
              placeholder={t("common.optional")}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              className="max-h-48"
            />
          </Field>
          <Field data-invalid={!!iconError}>
            <FieldLabel htmlFor={`${field}-icon`}>{t("icons.label")}</FieldLabel>
            <IconPicker
              id={`${field}-icon`}
              value={icon}
              onChange={setIcon}
              onError={setIconError}
            />
            <FieldError>{iconError}</FieldError>
          </Field>
        </FieldGroup>
      ),
    },
    ...(groups.length
      ? [
          {
            id: "options",
            label: t("build.options"),
            icon: RiEqualizerLine,
            group: image,
            content: <BuildOptions groups={groups} values={values} onChange={choose} hint />,
          },
        ]
      : []),
    {
      id: "script",
      label: t("build.script"),
      icon: RiTerminalBoxLine,
      group: build,
      content: (
        <FieldGroup>
          <SwitchTile
            icon={RiGlobalLine}
            label={t("build.internet")}
            checked={internet}
            onCheckedChange={setInternet}
            className="sm:max-w-sm"
          />
          {parts ? (
            <BuildParts
              parts={parts}
              // The editor hands back several parts at once, so each change builds on the last.
              onChange={(index, next) =>
                setParts(
                  (current) =>
                    current &&
                    current.map((part, other) =>
                      other === index ? { ...part, script: next } : part
                    )
                )
              }
              onMove={(index, by) => {
                const next = [...parts]
                ;[next[index], next[index + by]] = [next[index + by], next[index]]
                moveParts(next)
              }}
              onRemove={removePart}
            />
          ) : (
            <Field>
              {/* The section's heading names it; the label still leads the browser to it. */}
              <FieldLabel htmlFor={`${field}-script`} className="sr-only">
                {t("build.script")}
              </FieldLabel>
              <ScriptField
                id={`${field}-script`}
                label={t("build.script")}
                value={script}
                onChange={setScript}
                required
                hint={t("build.scriptHint")}
                completions={BUILD_SCRIPT}
              />
              <FieldDescription className="text-xs">{t("build.scriptHint")}</FieldDescription>
            </Field>
          )}
        </FieldGroup>
      ),
    },
    {
      id: "compose",
      label: t("compose.name"),
      icon: RiListCheck3,
      group: build,
      content: (
        <div className="flex flex-col gap-5">
          {baseImage && (
            <BuildBase base={baseImage} checked={inherit} onCheckedChange={setInherit} />
          )}
          {compose ? (
            <>
              <ComposeFields draft={compose} onChange={setCompose} />
              <Button
                type="button"
                variant="outline"
                className="self-start text-destructive"
                onClick={() => setCompose(null)}
              >
                {t("compose.remove")}
              </Button>
            </>
          ) : (
            <div className="flex flex-col items-start gap-3">
              <p className="text-sm text-muted-foreground">{t("compose.about")}</p>
              <Button type="button" variant="outline" onClick={() => setCompose(composeDraft())}>
                {t("compose.add")}
              </Button>
            </div>
          )}
        </div>
      ),
    },
    ...(kept.length
      ? [
          {
            id: "servers",
            label: t("build.mcp"),
            icon: RiPlugLine,
            group: build,
            content: (
              <BuildServers
                servers={kept}
                onRemove={(name) => setServers(servers.filter((server) => server.name !== name))}
              />
            ),
          },
        ]
      : []),
  ]
  const named = parts ? formatList(parts.map((part) => part.name)) : ""

  return (
    <Dialog {...props}>
      <DialogContent className={sectionedDialog}>
        {importing ? (
          <ImportConfiguration onLoad={load} onBack={() => setImporting(false)} />
        ) : (
          <form
            className="flex min-h-0 flex-1 flex-col"
            onSubmit={submit(() => {
              // The options go as values in the scripts; the request never names them.
              const build = resolveOptions({ script, parts, options, servers }, values)
              return runJob("images/build", {
                name,
                ...(description.trim() ? { description } : {}),
                base_image: chosen,
                ...(build.parts ? { parts: build.parts } : { script: build.script }),
                internet,
                ...(compose ? { compose: composeBody(compose) } : {}),
                ...(inherit ? {} : { inherit_compose: false }),
                ...(build.mcp.length ? { mcp: build.mcp } : {}),
                ...(icon ? { icon } : {}),
                ...(templateIds.length
                  ? parts
                    ? { template_ids: templateIds }
                    : { template_id: templateIds[0] }
                  : {}),
              })
            })}
          >
            <DialogHeader className="px-6 pt-6 pb-4">
              <DialogTitle>{t("images.build")}</DialogTitle>
              <DialogDescription>
                {simple
                  ? parts
                    ? t("build.simpleMerged", { names: named })
                    : t("build.simple")
                  : parts
                    ? t("build.fromTemplates", { names: named })
                    : templateIds.length
                      ? t("build.fromTemplate")
                      : t("build.description")}
              </DialogDescription>
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
              <Footer busy={busy} label={t("images.build")} disabled={!chosen}>
                {!simple && (
                  <Button
                    type="button"
                    variant="outline"
                    className="sm:mr-auto"
                    onClick={() => setImporting(true)}
                  >
                    <RiClipboardLine data-icon="inline-start" />
                    {t("build.importConfig")}
                  </Button>
                )}
              </Footer>
            </div>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}

/** The build dialog's paste view: a configuration copied from a built image fills the form. */
function ImportConfiguration({
  onLoad,
  onBack,
}: {
  onLoad: (config: ImportedBuild) => void
  onBack: () => void
}) {
  const [text, setText] = React.useState("")
  const [error, setError] = React.useState<string>()
  const field = React.useId()
  return (
    <form
      className="flex min-h-0 flex-1 flex-col gap-4 p-6"
      onSubmit={(event) => {
        event.preventDefault()
        try {
          onLoad(parseBuildConfig(text))
        } catch (reason) {
          setError((reason as Error).message)
        }
      }}
    >
      <DialogHeader>
        <DialogTitle>{t("build.importConfig")}</DialogTitle>
        <DialogDescription>{t("build.importConfigHint")}</DialogDescription>
      </DialogHeader>
      {/* The editor fills what the dialog leaves it, so the text scrolls in one place only. */}
      <FieldGroup className="min-h-0 flex-1">
        <Field data-invalid={!!error} className="min-h-0 flex-1">
          <FieldLabel htmlFor={`${field}-config`}>{t("build.config")}</FieldLabel>
          <CodeEditor
            id={`${field}-config`}
            language="json"
            label={t("build.config")}
            fill
            className="min-h-0 flex-1"
            required
            placeholder={'{\n  "name": "Unity tools",\n  "base_image": "…",\n  "script": "…"\n}'}
            value={text}
            onChange={(next) => {
              setText(next)
              setError(undefined)
            }}
            invalid={!!error}
            autoFocus
          />
          <FieldError>{error}</FieldError>
        </Field>
      </FieldGroup>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onBack}>
          {t("common.back")}
        </Button>
        <Button type="submit">{t("build.load")}</Button>
      </DialogFooter>
    </form>
  )
}
