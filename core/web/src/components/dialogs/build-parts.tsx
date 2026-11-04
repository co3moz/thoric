import * as React from "react"
import { RiArrowDownLine, RiArrowUpLine, RiDeleteBinLine } from "@remixicon/react"

import { Button } from "@/components/ui/button"
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { CodeEditor } from "@/components/code-editor"
import { rowsHeight } from "@/lib/code-editor"
import { t } from "@/lib/i18n"
import type { BuildPart } from "@/lib/types"
import { NumberedStep, NumberedSteps } from "./numbered-steps"

/**
 * The scripts of templates merged into one build, numbered in the order they run, each open to be
 * read and changed, moved or left out before anything is built.
 */
export function BuildParts({
  parts,
  onChange,
  onMove,
  onRemove,
}: {
  parts: BuildPart[]
  onChange: (index: number, script: string) => void
  onMove: (index: number, by: -1 | 1) => void
  onRemove: (index: number) => void
}) {
  const id = React.useId()
  return (
    <div className="flex flex-col gap-4">
      <FieldDescription className="text-xs">{t("build.partsHint")}</FieldDescription>
      <NumberedSteps>
        {parts.map((part, index) => (
          <NumberedStep
            key={part.id}
            number={index + 1}
            last={index === parts.length - 1}
            heading={
              <>
                <span className="min-w-0 truncate font-medium">{part.name}</span>
                <span className="ml-auto flex items-center gap-0.5">
                  <PartButton
                    label={t("build.partEarlier")}
                    disabled={index === 0}
                    onClick={() => onMove(index, -1)}
                  >
                    <RiArrowUpLine />
                  </PartButton>
                  <PartButton
                    label={t("build.partLater")}
                    disabled={index === parts.length - 1}
                    onClick={() => onMove(index, 1)}
                  >
                    <RiArrowDownLine />
                  </PartButton>
                  <PartButton
                    label={t("build.partRemoveOf", { name: part.name })}
                    disabled={parts.length === 1}
                    onClick={() => onRemove(index)}
                    destructive
                  >
                    <RiDeleteBinLine />
                  </PartButton>
                </span>
              </>
            }
          >
            <Field>
              <FieldLabel htmlFor={`${id}-${part.id}`} className="sr-only">
                {t("build.partScript", { name: part.name })}
              </FieldLabel>
              <CodeEditor
                id={`${id}-${part.id}`}
                language="bash"
                label={t("build.partScript", { name: part.name })}
                minHeight={rowsHeight(8)}
                maxHeight={400}
                required
                value={part.script}
                onChange={(script) => onChange(index, script)}
              />
            </Field>
          </NumberedStep>
        ))}
      </NumberedSteps>
    </div>
  )
}

/** A part's icon button, named by its tooltip. */
function PartButton({
  label,
  disabled,
  onClick,
  destructive,
  children,
}: {
  label: string
  disabled: boolean
  onClick: () => void
  destructive?: boolean
  children: React.ReactNode
}) {
  const button = (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
      className={destructive ? "text-destructive" : "text-muted-foreground"}
    >
      {children}
    </Button>
  )
  // A disabled button gets no hover, so it shows no tooltip either.
  if (disabled) return button
  return (
    <Tooltip>
      <TooltipTrigger asChild>{button}</TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
