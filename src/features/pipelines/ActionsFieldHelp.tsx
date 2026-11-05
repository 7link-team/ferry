import { type ReactNode } from "react";
import { fieldLabelText } from "../../shared/forms/FieldHelp";
import { executionText as e, workflowText as t } from "../../shared/i18n/i18n";

export function actionsFieldHelp(label: ReactNode): ReactNode {
  const name = fieldLabelText(label);
  const fields: [string[], ReactNode][] = [
    [[t("actions.stepId")], t("help.stepId")],
    [[t("runCondition")], t("actions.conditionStepHelp") + " " + t("actions.conditionJobHelp")],
    [[t("environmentVariables")], t("help.env")],
    [[t("actions.shell"), "Shell"], t("actions.shellHelp")],
    [[t("workingDirectory")], t("help.directory")],
    [[e("shared.resource.cron.commandAction")], t("help.command")],
    [[t("actions.commonAction"), e("editor.actionReference"), t("actions.reusableReference")], t("help.action")],
    [[e("editor.parameters"), t("actions.customInputs"), t("actions.workflowInputs")], t("help.parameters")],
    [[e("editor.targetType"), e("editor.executionHost"), e("editor.executionHostGroup"), e("editor.runMode"), e("editor.executionImage"), e("editor.executorLabels"), t("actions.runnerGroup")], t("help.executor")],
    [[e("editor.dependencies")], t("help.dependencies")],
    [[e("editor.timeout")], t("help.timeout")],
    [[t("continueOnError")], t("help.continue")],
    [[t("actions.jobOutputs")], t("actions.jobOutputsHelp")],
    [[e("editor.stepName"), e("shared.common.common.name"), t("actions.workflowName")], t("help.name")],
  ];
  return fields.find(([labels]) => labels.includes(name))?.[1] ?? null;
}
