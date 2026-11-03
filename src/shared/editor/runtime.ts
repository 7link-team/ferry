import { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
import JsonWorker from "monaco-editor/esm/vs/language/json/json.worker.js?worker";
import TsWorker from "monaco-editor/esm/vs/language/typescript/ts.worker.js?worker";
import HtmlWorker from "monaco-editor/esm/vs/language/html/html.worker.js?worker";
import CssWorker from "monaco-editor/esm/vs/language/css/css.worker.js?worker";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker.js?worker";
import { configureMonacoYaml } from "monaco-yaml";
import YamlWorker from "./yaml.worker?worker";

globalThis.MonacoEnvironment = {
  getWorker(_moduleId, label) {
    if (label === "yaml") return new YamlWorker();
    if (label === "json") return new JsonWorker();
    if (label === "typescript" || label === "javascript") return new TsWorker();
    if (["html", "handlebars", "razor"].includes(label)) return new HtmlWorker();
    if (["css", "scss", "less"].includes(label)) return new CssWorker();
    return new EditorWorker();
  },
};
loader.config({ monaco });
configureMonacoYaml(monaco, { enableSchemaRequest: false });
