import Editor from "@monaco-editor/react";
import "./runtime";
import { useTheme } from "../theme/theme";

/** Shared local Monaco runtime for workflow definitions and remote text files. */
export function CodeEditor({ value, language, path, label, readOnly = false, onChange }: {
  value: string; language: string; path: string; label: string; readOnly?: boolean; onChange: (value: string) => void;
}) {
  const { resolved } = useTheme();
  return <Editor path={path} language={language} theme={resolved === "dark" ? "vs-dark" : "vs"}
    value={value} onChange={next => onChange(next ?? "")}
    options={{ occurrencesHighlight: "off", ariaLabel: label, readOnly, automaticLayout: true,
      // Use Monaco's established textarea input path for controlled React models.
      // Chromium EditContext can lose rapid keystrokes when these models update.
      editContext: false,
      minimap: { enabled: false }, scrollBeyondLastLine: false, fontSize: 13, tabSize: 2, wordWrap: "on" }} />;
}
