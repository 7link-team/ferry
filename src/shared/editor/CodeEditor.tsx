import { LoadingSkeleton } from "../ui/loading-skeleton";
import Editor, { DiffEditor } from "@monaco-editor/react";
import { useEffect, useImperativeHandle, useLayoutEffect, useRef, type Ref } from "react";
import type { editor as MonacoEditor } from "monaco-editor";
import "./runtime";
import { useTheme } from "../theme/theme";

/** Shared local Monaco runtime for workflow definitions and remote text files. */
/** A problem shown in the editor at its line and column (1-based). */
export interface EditorMarker { line: number; column: number; message: string }

export function CodeEditor({ value, language, path, label, readOnly = false, originalValue, onChange, insertRef, markers }: {
  value: string; language: string; path: string; label: string; readOnly?: boolean; originalValue?: string; onChange: (value: string) => void;
  insertRef?: Ref<{ insertText(value: string): void }>;
  markers?: EditorMarker[] | undefined;
}) {
  const { resolved } = useTheme();
  const change = useRef(onChange);
  change.current = onChange;
  const subscription = useRef<{ dispose(): void } | undefined>(undefined);
  const diffEditor = useRef<MonacoEditor.IStandaloneDiffEditor | undefined>(undefined);
  const sourceEditor = useRef<MonacoEditor.IStandaloneCodeEditor | undefined>(undefined);
  const monaco = useRef<typeof import("monaco-editor") | undefined>(undefined);
  const latest = useRef(markers);
  latest.current = markers;
  const applyMarkers = () => {
    const model = sourceEditor.current?.getModel();
    const runtime = monaco.current;
    if (!runtime || !model) return;
    runtime.editor.setModelMarkers(model, "ferry", (latest.current ?? []).map(marker => {
      const line = Math.min(Math.max(marker.line, 1), model.getLineCount());
      return { severity: runtime.MarkerSeverity.Error, message: marker.message,
        startLineNumber: line, startColumn: Math.max(marker.column, 1), endLineNumber: line, endColumn: model.getLineMaxColumn(line) };
    }));
  };
  const markerKey = JSON.stringify(markers ?? []);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(applyMarkers, [markerKey]);
  useImperativeHandle(insertRef, () => ({ insertText(text: string) {
    const editor = sourceEditor.current;
    const selection = editor?.getSelection();
    if (readOnly || !editor || !selection) return;
    editor.focus();
    editor.pushUndoStop();
    editor.executeEdits("insert-variable", [{ range: selection, text, forceMoveMarkers: true }]);
    editor.pushUndoStop();
  } }), [readOnly]);
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
  if (originalValue !== undefined) return <DiffEditor loading={<LoadingSkeleton variant="editor" />} originalModelPath={`${path}.original`} modifiedModelPath={path}
    original={originalValue} modified={value} language={language} theme={resolved === "dark" ? "vs-dark" : "vs"}
    onMount={editor => { diffEditor.current = editor; subscription.current?.dispose(); subscription.current = editor.getModifiedEditor().onDidChangeModelContent(() => change.current(editor.getModifiedEditor().getValue())); }}
    options={{ occurrencesHighlight: "off", ariaLabel: label, editContext: false, readOnly, originalEditable: false, automaticLayout: true, renderSideBySide: true, useInlineViewWhenSpaceIsLimited: false,
      minimap: { enabled: false }, scrollBeyondLastLine: false, fontSize: 13, wordWrap: "on", renderOverviewRuler: false }} />;
  return <Editor loading={<LoadingSkeleton variant="editor" />} path={path} language={language} theme={resolved === "dark" ? "vs-dark" : "vs"}
    onMount={(editor, runtime) => { sourceEditor.current = editor; monaco.current = runtime as typeof import("monaco-editor"); applyMarkers(); }}
    value={value} onChange={next => onChange(next ?? "")}
    options={{ occurrencesHighlight: "off", ariaLabel: label, readOnly, automaticLayout: true,
      // Use Monaco's established textarea input path for controlled React models.
      // Chromium EditContext can lose rapid keystrokes when these models update.
      editContext: false,
      minimap: { enabled: false }, scrollBeyondLastLine: false, fontSize: 13, tabSize: 2, wordWrap: "on" }} />;
}
