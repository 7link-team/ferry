import { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker.js?worker";
import { configureMonacoYaml } from "monaco-yaml";
import YamlWorker from "./yaml.worker?worker";

globalThis.MonacoEnvironment = {
  getWorker(_moduleId, label) {
    return label === "yaml" ? new YamlWorker() : new EditorWorker();
  },
};
loader.config({ monaco });
configureMonacoYaml(monaco, { enableSchemaRequest: false });
