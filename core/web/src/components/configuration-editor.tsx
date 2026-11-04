import * as React from "react"
import {
  RiAlertLine,
  RiArrowGoBackLine,
  RiDeleteBinLine,
  RiInformationLine,
  RiLockLine,
  RiTerminalBoxLine,
} from "@remixicon/react"
import { cn } from "cn"
import { toast } from "sonner"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { FieldError } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { api } from "@/lib/api"
import { openDialog, whenDiscarded } from "@/lib/dialogs"
import { formatList, has, plural, t } from "@/lib/i18n"
import { queryClient, useConfiguration, useSettings } from "@/lib/queries"
import {
  changesOf,
  choiceIndex,
  fieldOf,
  gpuNotes,
  idleSettings,
  initialText,
  isOn,
  sections,
  settingRows,
  shownValue,
  textProblem,
} from "@/lib/settings"
import type {
  ConfigVariable,
  Configuration,
  ConfigurationCheck,
  Job,
  JobStarted,
} from "@/lib/types"
import { followRestart } from "@/lib/update"

/** The job of a settings change the page reloaded from, for the notice after the reload. */
const SETTINGS_JOB = "thoric.settingsJob"

function storeJob(id: string) {
  try {
    sessionStorage.setItem(SETTINGS_JOB, id)
  } catch {
    // The notice after the reload is a convenience.
  }
}

/** A record without one of its keys. */
function without<T>(record: Record<string, T>, key: string) {
  const rest = { ...record }
  delete rest[key]
  return rest
}

function takeJob() {
  try {
    const id = sessionStorage.getItem(SETTINGS_JOB)
    sessionStorage.removeItem(SETTINGS_JOB)
    return id
  } catch {
    return null
  }
}

/** A setting's value as the manager runs with it: hidden, not set, or the text `.env` would hold. */
function Value({ variable }: { variable: ConfigVariable }) {
  const shown = shownValue(variable)
  if (shown.kind === "secret")
    return (
      <span className="inline-flex items-center gap-1.5 text-muted-foreground">
        <RiLockLine className="size-3.5 shrink-0" />
        {shown.set ? t("settings.secretSet") : t("settings.notSet")}
      </span>
    )
  if (shown.kind === "empty")
    return <span className="text-muted-foreground">{t("settings.notSet")}</span>
  return <span className="font-mono text-xs break-all">{shown.text}</span>
}

/** What GPU desktops get on this host, and why not a GPU their settings allow. */
function GpuStatus() {
  const report = useSettings().data?.gpu
  if (!report) return null
  const { routes, unused } = gpuNotes(report)
  return (
    <div className="flex flex-col gap-1 pb-2 text-xs">
      <p className={routes.length ? "" : "font-medium text-warning"}>
        {routes.length
          ? t("settings.gpu.uses", {
              routes: formatList(routes.map((route) => t(`settings.gpu.route.${route}`))),
            })
          : t("settings.gpu.none")}
      </p>
      {unused.map(({ setting, reason }) => (
        <p key={setting} className="text-muted-foreground">
          {t("settings.gpu.unused", { setting, reason: t(`settings.gpu.reason.${reason}`) })}
        </p>
      ))}
    </div>
  )
}

/** Marks a setting that changes only in `.env`, with the command that does it. */
function CliOnly() {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Badge variant="outline" className="px-1.5 text-muted-foreground" tabIndex={0}>
          <RiTerminalBoxLine />
          {t("settings.edit.cliOnly")}
        </Badge>
      </TooltipTrigger>
      <TooltipContent>{t("settings.edit.cliOnlyHint")}</TooltipContent>
    </Tooltip>
  )
}

type Edit = {
  text: string
  /** A choice field shows its own text's input, whatever that text is. */
  other: boolean
  error?: string
  disabled: boolean
  onChange: (text: string, other?: boolean) => void
  /** A secret, whose value the page never has: only a new one, or its removal, is typed here. */
  secret?: { set: boolean; removing: boolean; onRemove: () => void }
}

/**
 * A secret's field: empty while it stays as it is, a new value that replaces it, or removed. What
 * it holds now is never shown, since the page never gets it.
 */
function SecretControl({
  name,
  edit,
  secret,
}: {
  name: string
  edit: Edit
  secret: NonNullable<Edit["secret"]>
}) {
  const remove = t("settings.edit.secretRemove", { name })
  return (
    <div className="flex w-full items-center gap-1 @md/settings:justify-end">
      <Input
        id={`setting-${name}`}
        aria-label={name}
        aria-invalid={edit.error ? true : undefined}
        type="password"
        value={edit.text}
        placeholder={
          secret.removing
            ? t("settings.edit.secretRemoving")
            : secret.set
              ? t("settings.edit.secretReplace")
              : t("settings.notSet")
        }
        spellCheck={false}
        autoComplete="new-password"
        disabled={edit.disabled || secret.removing}
        onChange={(event) => edit.onChange(event.target.value)}
        className="min-w-0 flex-1 font-mono text-xs"
      />
      {secret.set && !secret.removing && edit.text === "" && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              className="shrink-0 text-muted-foreground"
              aria-label={remove}
              disabled={edit.disabled}
              onClick={secret.onRemove}
            >
              <RiDeleteBinLine />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{remove}</TooltipContent>
        </Tooltip>
      )}
    </div>
  )
}

/**
 * The control of an editable setting, by the kind of value it takes. While the switch it needs is
 * off it reads as off and cannot change, though the draft keeps its text for when that comes on.
 */
function Control({ name, edit, idle }: { name: string; edit: Edit; idle: boolean }) {
  const field = fieldOf(name)
  const id = `setting-${name}`
  const invalid = edit.error ? true : undefined
  if (edit.secret) return <SecretControl name={name} edit={edit} secret={edit.secret} />
  if (field.kind === "switch")
    return (
      <Switch
        id={id}
        aria-label={name}
        aria-describedby={idle ? `${id}-idle` : undefined}
        checked={!idle && isOn(edit.text)}
        disabled={edit.disabled || idle}
        onCheckedChange={(on) => edit.onChange(on ? "true" : "false")}
      />
    )
  if (field.kind === "number")
    return (
      <div className="flex items-center gap-2 @md/settings:justify-end">
        <Input
          id={id}
          aria-label={name}
          aria-invalid={invalid}
          type="number"
          inputMode="numeric"
          min={field.min}
          max={field.max}
          step={1}
          value={edit.text}
          disabled={edit.disabled}
          onChange={(event) => edit.onChange(event.target.value)}
          className="w-32 font-mono text-xs"
        />
        <span className="text-xs text-muted-foreground">{t("settings.edit.mib")}</span>
      </div>
    )
  const input = (placeholder?: string) => (
    <Input
      id={id}
      aria-label={name}
      aria-invalid={invalid}
      value={edit.text}
      placeholder={placeholder}
      spellCheck={false}
      autoComplete="off"
      disabled={edit.disabled}
      onChange={(event) => edit.onChange(event.target.value, true)}
      className="font-mono text-xs"
    />
  )
  if (field.kind === "text") return input(field.placeholder)
  const index = edit.other ? -1 : choiceIndex(field, edit.text)
  return (
    <div className="flex flex-col gap-2 @md/settings:items-end">
      <Select
        value={index < 0 ? "other" : `choice-${index}`}
        disabled={edit.disabled}
        onValueChange={(value) => {
          if (value === "other") edit.onChange(index < 0 ? edit.text : "", true)
          else edit.onChange(field.choices[Number(value.slice(7))].value, false)
        }}
      >
        <SelectTrigger aria-label={name} className="w-full @md/settings:w-56">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            {field.choices.map((choice, i) => (
              <SelectItem key={choice.value} value={`choice-${i}`}>
                {choice.label ? t(choice.label) : choice.value}
              </SelectItem>
            ))}
            <SelectItem value="other">{t("settings.choice.custom")}</SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>
      {index < 0 && input(field.placeholder)}
    </div>
  )
}

/**
 * One setting: its name, what it does, and its value or the control that changes it. Half of a
 * pair, it keeps its value beside its name at every width.
 */
function SettingRow({
  variable,
  edit,
  changed,
  cliOnly,
  idle,
  paired,
  className,
  onReset,
}: {
  variable: ConfigVariable
  edit?: Edit
  changed: boolean
  cliOnly: boolean
  /** The switch that is off, which leaves this setting without effect. */
  idle?: string
  paired?: boolean
  className?: string
  onReset: () => void
}) {
  const about = `settings.about.${variable.name}`
  return (
    <dl
      data-changed={changed || undefined}
      className={cn(
        "grid",
        paired
          ? "grid-cols-[minmax(0,1fr)_auto] gap-3"
          : "gap-1.5 @md/settings:grid-cols-[minmax(0,1fr)_minmax(0,1fr)] @md/settings:gap-4",
        className
      )}
    >
      <dt className="flex min-w-0 flex-col gap-0.5">
        <span className="flex flex-wrap items-center gap-1.5">
          {edit ? (
            <label
              htmlFor={`setting-${variable.name}`}
              className="font-mono text-xs font-medium break-all"
            >
              {variable.name}
            </label>
          ) : (
            <code className="font-mono text-xs font-medium break-all">{variable.name}</code>
          )}
          {changed && (
            <Badge variant="secondary" className="px-1.5 text-edit">
              {t("settings.edit.changed")}
            </Badge>
          )}
          {cliOnly && <CliOnly />}
        </span>
        {has(about) && <span className="text-xs text-muted-foreground">{t(about)}</span>}
        {idle && (
          <span
            id={`setting-${variable.name}-idle`}
            className="text-xs font-medium text-muted-foreground"
          >
            {t("settings.idle", { name: idle })}
          </span>
        )}
      </dt>
      <dd
        className={cn(
          "flex min-w-0 flex-col gap-1 text-sm",
          paired ? "items-end text-right" : "@md/settings:items-end @md/settings:text-right"
        )}
      >
        {edit ? (
          <div
            className={cn("flex items-start gap-1", !paired && "w-full @md/settings:justify-end")}
          >
            <div
              className={
                paired ? "flex" : "min-w-0 flex-1 @md/settings:flex @md/settings:justify-end"
              }
            >
              <Control name={variable.name} edit={edit} idle={!!idle} />
            </div>
            {changed && (
              <Button
                variant="ghost"
                size="icon-sm"
                className="shrink-0 text-muted-foreground"
                aria-label={t("settings.edit.reset", { name: variable.name })}
                disabled={edit.disabled}
                onClick={onReset}
              >
                <RiArrowGoBackLine />
              </Button>
            )}
          </div>
        ) : (
          <Value variable={variable} />
        )}
        {edit?.error && <FieldError className="text-xs">{edit.error}</FieldError>}
      </dd>
    </dl>
  )
}

/** Asks before the WSL package writes the changes to `.env` and recreates the manager. */
function ApplyDialog({
  pending,
  variables,
  onClose,
  onStarted,
}: {
  pending: { changes: Record<string, string>; running: number } | null
  variables: ConfigVariable[]
  onClose: () => void
  onStarted: (job: string) => void
}) {
  const [busy, setBusy] = React.useState(false)
  const [error, setError] = React.useState<string>()
  const variableOf = (name: string) => variables.find((entry) => entry.name === name)
  const apply = async (event: React.FormEvent) => {
    event.preventDefault()
    if (!pending) return
    setBusy(true)
    setError(undefined)
    try {
      const { job_id } = await api<JobStarted>("configuration", {
        changes: pending.changes,
        interrupt_jobs: pending.running > 0,
      })
      onStarted(job_id)
    } catch (failure) {
      setError((failure as Error).message)
    } finally {
      setBusy(false)
    }
  }
  const shown = (text: string) =>
    text === "" ? <span className="text-muted-foreground">{t("settings.notSet")}</span> : text
  // A secret reads as whether it is set, before and after: the page never has its value, and
  // never repeats the one typed.
  const before = (name: string) => {
    const variable = variableOf(name)
    if (variable?.secret)
      return (
        <span className="text-muted-foreground">
          {variable.set ? t("settings.secretSet") : t("settings.notSet")}
        </span>
      )
    return shown(variable ? initialText(variable) : "")
  }
  const after = (name: string, text: string) =>
    variableOf(name)?.secret && text !== "" ? t("settings.edit.secretNew") : shown(text)
  return (
    <AlertDialog open={!!pending} onOpenChange={(open) => !open && !busy && onClose()}>
      <AlertDialogContent>
        <form className="contents" onSubmit={(event) => void apply(event)}>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.edit.confirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("settings.edit.confirmText")}</AlertDialogDescription>
          </AlertDialogHeader>
          {pending && (
            <ul className="flex max-h-60 flex-col gap-1.5 overflow-auto rounded-xl border p-3 text-xs">
              {Object.entries(pending.changes).map(([name, text]) => (
                <li key={name} className="flex flex-col gap-0.5">
                  <code className="font-mono font-medium">{name}</code>
                  <span className="font-mono break-all text-muted-foreground">
                    {before(name)} {"→"}{" "}
                    <span className="text-foreground">{after(name, text)}</span>
                  </span>
                </li>
              ))}
            </ul>
          )}
          {pending && pending.running > 0 && (
            <Alert variant="destructive">
              <RiAlertLine />
              <AlertDescription>
                {plural(pending.running, {
                  one: "settings.edit.jobs.one",
                  other: "settings.edit.jobs.other",
                })}
              </AlertDescription>
            </Alert>
          )}
          <FieldError>{error}</FieldError>
          <AlertDialogFooter>
            <AlertDialogCancel type="button" disabled={busy}>
              {t("common.cancel")}
            </AlertDialogCancel>
            <Button
              type="submit"
              variant={pending && pending.running > 0 ? "destructive" : "default"}
              disabled={busy}
            >
              {busy && <Spinner data-icon="inline-start" />}
              {pending && pending.running > 0
                ? t("settings.edit.confirmJobs")
                : t("settings.edit.apply")}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** Sign-in or the MCP token off while the manager is published beyond loopback. */
function ExposureAlert({ exposure }: { exposure: NonNullable<Configuration["exposure"]> }) {
  const dashboard = exposure.open.includes("dashboard")
  const mcp = exposure.open.includes("mcp")
  const key =
    dashboard && mcp
      ? "settings.exposure.both"
      : dashboard
        ? "settings.exposure.dashboard"
        : "settings.exposure.mcp"
  return (
    <Alert variant="destructive">
      <RiAlertLine />
      <AlertTitle>{t("settings.exposure.title")}</AlertTitle>
      <AlertDescription>
        <p>{t(key, { addresses: formatList(exposure.published) })}</p>
        <p>{t("settings.exposure.fix")}</p>
      </AlertDescription>
    </Alert>
  )
}

/** What each gap of an older Docker Engine means for desktops. */
const DOCKER_LACKS = {
  isolation: "settings.docker.isolation",
  mac_address: "settings.docker.mac_address",
} as const

/**
 * What the Docker Engine lacks of what Thoric uses, and who updates it: the WSL package's
 * `thoric system`, or whoever runs the Docker host.
 */
function DockerEngineAlert({ docker }: { docker: NonNullable<Configuration["docker"]> }) {
  return (
    <Alert>
      <RiAlertLine />
      <AlertTitle>{t("settings.docker.title", { version: docker.version })}</AlertTitle>
      <AlertDescription>
        {docker.lacks.map((gap) => (
          <p key={gap}>{t(DOCKER_LACKS[gap])}</p>
        ))}
        <p>{t(docker.package ? "settings.docker.package" : "settings.docker.host")}</p>
      </AlertDescription>
    </Alert>
  )
}

/**
 * The installation's settings by section. Where the WSL package takes changes, the editable ones
 * get a control, and **Apply and restart** has the package write them to `.env` and recreate
 * the manager; the rest are marked as changing with `thoric settings`.
 */
export function ConfigurationEditor() {
  const configuration = useConfiguration()
  const data = configuration.data
  const job = data?.job_id ?? null
  const [draft, setDraft] = React.useState<Record<string, string>>({})
  const [other, setOther] = React.useState<Record<string, boolean>>({})
  const [errors, setErrors] = React.useState<Record<string, string>>({})
  const [together, setTogether] = React.useState<string>()
  const [checking, setChecking] = React.useState(false)
  const [pending, setPending] = React.useState<{
    changes: Record<string, string>
    running: number
  } | null>(null)

  // A change outlives the manager it restarts; the page follows it to the next one.
  React.useEffect(() => {
    if (job) followRestart("settings", "", () => storeJob(job))
  }, [job])
  React.useEffect(() => {
    const id = takeJob()
    if (!id) return
    api<Job>(`jobs/${id}`)
      .then((finished) => {
        if (finished.status === "completed") toast.success(t("settings.edit.done"))
        else if (finished.status === "failed")
          toast.error(t("settings.edit.failed"), {
            action: {
              label: t("update.showLog"),
              onClick: () => whenDiscarded(() => openDialog({ kind: "job", id })),
            },
          })
      })
      .catch(() => {})
  }, [])

  if (configuration.error)
    return (
      <div className="px-4 lg:px-6">
        <Alert variant="destructive">
          <RiAlertLine />
          <AlertTitle>{t("settings.configurationFailed")}</AlertTitle>
          <AlertDescription>{configuration.error.message}</AlertDescription>
        </Alert>
      </div>
    )
  if (!data)
    return (
      <div className="px-4 lg:px-6">
        <Skeleton className="h-72 rounded-2xl" />
      </div>
    )

  const editable = !!data.editable
  const canEdit = (variable: ConfigVariable) => editable && !!variable.editable
  const textOf = (variable: ConfigVariable) => draft[variable.name] ?? initialText(variable)
  // A switch reads as the draft has it, so a default goes idle as soon as its feature is off.
  const idle = idleSettings(data.variables, (name) => {
    const variable = data.variables.find((entry) => entry.name === name)
    if (!variable || variable.secret) return false
    return canEdit(variable) ? isOn(textOf(variable)) : variable.value === true
  })
  const changes = changesOf(data.variables, draft)
  const count = Object.keys(changes).length
  const problem = (name: string) => {
    if (!(name in changes)) return undefined
    const found = textProblem(name, changes[name])
    return found ? t(found.key, found.values) : errors[name]
  }
  const set = (variable: ConfigVariable, text: string, custom?: boolean) => {
    setDraft((before) => ({ ...before, [variable.name]: text }))
    if (custom !== undefined) setOther((before) => ({ ...before, [variable.name]: custom }))
    setErrors((before) => without(before, variable.name))
    setTogether(undefined)
  }
  const reset = (name: string) => {
    setDraft((before) => without(before, name))
    setOther((before) => without(before, name))
    setErrors((before) => without(before, name))
    setTogether(undefined)
  }
  const discard = () => {
    setDraft({})
    setOther({})
    setErrors({})
    setTogether(undefined)
  }
  const check = async () => {
    if (Object.keys(changes).some((name) => textProblem(name, changes[name]))) {
      toast.error(t("settings.edit.fix"))
      return
    }
    setChecking(true)
    try {
      const result = await api<ConfigurationCheck>("configuration", { changes, dry_run: true })
      setErrors(result.errors)
      setTogether(result.error ?? undefined)
      if (Object.keys(result.errors).length || result.error) toast.error(t("settings.edit.fix"))
      else setPending({ changes, running: result.running_jobs })
    } catch (failure) {
      toast.error((failure as Error).message)
    } finally {
      setChecking(false)
    }
  }
  const started = (id: string) => {
    setPending(null)
    discard()
    queryClient.setQueryData(["configuration"], { ...data, job_id: id })
    openDialog({ kind: "job", id })
  }

  return (
    <>
      <div className="flex flex-col gap-3 px-4 lg:px-6">
        {data.exposure && <ExposureAlert exposure={data.exposure} />}
        {data.docker && <DockerEngineAlert docker={data.docker} />}
        {job ? (
          <Alert>
            <Spinner />
            <AlertTitle>{t("settings.edit.running")}</AlertTitle>
            <AlertDescription className="flex flex-col items-start gap-2">
              {t("settings.edit.runningDetail")}
              <Button
                variant="outline"
                size="sm"
                onClick={() => openDialog({ kind: "job", id: job })}
              >
                {t("update.showLog")}
              </Button>
            </AlertDescription>
          </Alert>
        ) : (
          <Alert>
            <RiInformationLine />
            <AlertDescription>
              {editable ? t("settings.edit.editableHint") : t("settings.edit.readOnly")}
            </AlertDescription>
          </Alert>
        )}
      </div>
      <div className="grid grid-cols-1 items-start gap-4 px-4 lg:px-6 @5xl/main:grid-cols-2">
        {sections(data.variables).map(({ section, variables }) => (
          <Card key={section} size="sm">
            <CardHeader>
              <CardTitle className="text-sm">{t(`settings.section.${section}`)}</CardTitle>
            </CardHeader>
            <CardContent className="@container/settings">
              {section === "gpu" && <GpuStatus />}
              <div className="flex flex-col divide-y">
                {settingRows(variables).map((row) => {
                  const paired = row.length > 1
                  const settings = row.map((variable) => {
                    const name = variable.name
                    return (
                      <SettingRow
                        key={name}
                        variable={variable}
                        changed={name in changes}
                        cliOnly={editable && !canEdit(variable)}
                        idle={idle[name]}
                        paired={paired}
                        className={paired ? undefined : "py-2.5 first:pt-0 last:pb-0"}
                        onReset={() => reset(name)}
                        edit={
                          canEdit(variable)
                            ? {
                                text: textOf(variable),
                                other: other[name] ?? false,
                                error: problem(name),
                                disabled: !!job,
                                // A secret's field emptied again leaves it as it is; only
                                // its remove button takes it away.
                                onChange: (text, custom) =>
                                  variable.secret && text === ""
                                    ? reset(name)
                                    : set(variable, text, custom),
                                secret: variable.secret
                                  ? {
                                      set: variable.set,
                                      removing: draft[name] === "",
                                      onRemove: () => set(variable, ""),
                                    }
                                  : undefined,
                              }
                            : undefined
                        }
                      />
                    )
                  })
                  if (!paired) return settings
                  return (
                    <div
                      key={row[0].name}
                      className="grid gap-3 py-2.5 first:pt-0 last:pb-0 @md/settings:grid-cols-2 @md/settings:gap-6"
                    >
                      {settings}
                    </div>
                  )
                })}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>
      {editable && count > 0 && !job && (
        <div className="sticky bottom-4 z-10 px-4 lg:px-6">
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border bg-card p-3 shadow-lg">
            <div className="flex min-w-0 flex-col gap-0.5">
              <span className="text-sm font-medium">
                {plural(count, {
                  one: "settings.edit.changes.one",
                  other: "settings.edit.changes.other",
                })}
              </span>
              {together && <FieldError className="text-xs">{together}</FieldError>}
            </div>
            <div className="flex flex-wrap gap-2">
              <Button variant="outline" size="sm" disabled={checking} onClick={discard}>
                {t("settings.edit.discard")}
              </Button>
              <Button size="sm" disabled={checking} onClick={() => void check()}>
                {checking && <Spinner data-icon="inline-start" />}
                {t("settings.edit.apply")}
              </Button>
            </div>
          </div>
        </div>
      )}
      <ApplyDialog
        pending={pending}
        variables={data.variables}
        onClose={() => setPending(null)}
        onStarted={started}
      />
    </>
  )
}
