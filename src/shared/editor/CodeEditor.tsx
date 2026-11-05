import Editor, { DiffEditor } from "@monaco-editor/react";
import { useEffect, useLayoutEffect, useRef } from "react";
import type { editor as MonacoEditor } from "monaco-editor";
import "./runtime";
import { useTheme } from "../theme/theme";

/** Shared local Monaco runtime for workflow definitions and remote text files. */
export function CodeEditor({ value, language, path, label, readOnly = false, originalValue, onChange }: {
  value: string; language: string; path: string; label: string; readOnly?: boolean; originalValue?: string; onChange: (value: string) => void;
}) {
  const { resolved } = useTheme();
  const change = useRef(onChange);
  change.current = onChange;
  const subscription = useRef<{ dispose(): void } | undefined>(undefined);
  const diffEditor = useRef<MonacoEditor.IStandaloneDiffEditor | undefined>(undefined);
  const comparing = originalValue !== undefined;
  useEffect(() => () => { subscription.current?.dispose(); subscription.current = undefined; }, [comparing]);
  useLayoutEffect(() => () => {
    // Detach before the React wrapper disposes models: Monaco diff views still
    // reference both models during their asynchronous diff/layout work.
    const editor = diffEditor.current;
    diffEditor.current = undefined;
    const models = editor?.getModel();
    subscription.current?.dispose(); subscription.current = undefined;
    editor?.setModel(null);
    models?.original.dispose();
    models?.modified.dispose();
  }, [comparing]);
  if (originalValue !== undefined) return <DiffEditor originalModelPath={`${path}.original`} modifiedModelPath={path}
    original={originalValue} modified={value} language={language} theme={resolved === "dark" ? "vs-dark" : "vs"}
    onMount={editor => { diffEditor.current = editor; subscription.current?.dispose(); subscription.current = editor.getModifiedEditor().onDidChangeModelContent(() => change.current(editor.getModifiedEditor().getValue())); }}
    options={{ ariaLabel: label, editContext: false, readOnly, originalEditable: false, automaticLayout: true, renderSideBySide: true, useInlineViewWhenSpaceIsLimited: false,
      minimap: { enabled: false }, scrollBeyondLastLine: false, fontSize: 13, wordWrap: "on", renderOverviewRuler: false }} />;
  return <Editor path={path} language={language} theme={resolved === "dark" ? "vs-dark" : "vs"}
    value={value} onChange={next => onChange(next ?? "")}
    options={{ occurrencesHighlight: "off", ariaLabel: label, readOnly, automaticLayout: true,
      // Use Monaco's established textarea input path for controlled React models.
      // Chromium EditContext can lose rapid keystrokes when these models update.
      editContext: false,
      minimap: { enabled: false }, scrollBeyondLastLine: false, fontSize: 13, tabSize: 2, wordWrap: "on" }} />;
}
