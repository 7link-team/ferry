import type { ReactNode } from "react";
import { fieldLabelText } from "../../shared/forms/FieldHelp";
import { workflowText as t } from "../../shared/i18n/i18n";

export function workflowFieldHelp(label: ReactNode): ReactNode {
  const name = fieldLabelText(label);
  const groups: [Parameters<typeof t>[0][], Parameters<typeof t>[0]][] = [
    [["sourceText", "valueType"], "expressionHelp"],
    [["inputFile", "binaryField", "uploadFile"], "localUploadHelp"],
    [["fileFormat", "fileValuePath"], "decodeHelp"],
    [["aiCredential", "aiCredentials", "aiProvider"], "integration.credentialHelp"],
    [["aiInputPrice", "aiOutputPrice", "aiPriceUnit", "aiCallBudget", "aiMonthlyBudget", "aiCurrency", "aiActualCost"], "aiPriceHelp"],
    [["aiPrompt", "aiImageField"], "nodeHint.aiChat"],
    [["aiImageSize"], "nodeHint.aiImage"],
    [["aiVoice"], "nodeHint.aiSpeech"],
    [["inspector.sample"], "inspector.sampleHelp"],
    [["advanced"], "advancedHelp"],
  ];
  const group = groups.find(([keys]) => keys.some(key => t(key) === name));
  return group ? t(group[1]) : null;
}
