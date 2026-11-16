import type { YAMLError } from "yaml";
import { i18n } from "./i18n/i18n";

/** A YAML syntax error worded for people; the parser's own text stays as `detail`. */
export class YamlSyntaxError extends Error {
  readonly line: number | undefined;
  readonly column: number | undefined;
  readonly detail: string;
  constructor(message: string, line: number | undefined, column: number | undefined, detail: string) {
    super(message);
    this.name = "YamlSyntaxError";
    this.line = line;
    this.column = column;
    this.detail = detail;
  }
}

function reason(code: YAMLError["code"] | undefined) {
  switch (code) {
    case "DUPLICATE_KEY":
      return i18n.t("errors.yamlDuplicate");
    case "TAB_AS_INDENT":
      return i18n.t("errors.yamlTab");
    case "BAD_INDENT":
    case "MULTILINE_IMPLICIT_KEY":
    case "BLOCK_AS_IMPLICIT_KEY":
      return i18n.t("errors.yamlIndent");
    case "MISSING_CHAR":
      return i18n.t("errors.yamlUnclosed");
    case "UNEXPECTED_TOKEN":
    case "BAD_SCALAR_START":
    case "BLOCK_IN_FLOW":
    case "BAD_PROP_ORDER":
      return i18n.t("errors.yamlToken");
    default:
      return i18n.t("errors.yamlOther");
  }
}

/** The first parser error as a localized message with its line and column (1-based). */
export function yamlError(errors: readonly YAMLError[]): YamlSyntaxError {
  const first = errors[0];
  const position = first?.linePos?.[0];
  const why = reason(first?.code);
  const message = position
    ? i18n.t("errors.yaml", { line: position.line, column: position.col, reason: why })
    : i18n.t("errors.yamlNoPosition", { reason: why });
  return new YamlSyntaxError(
    message,
    position?.line,
    position?.col,
    errors.map((error) => error.message).join("\n"),
  );
}
