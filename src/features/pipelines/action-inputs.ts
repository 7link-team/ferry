/** Parameter names are Action interfaces, not executable Action implementations.
 * Suggestions never add values to YAML until the user edits a field.
 */
export interface ActionInput { key: string; placeholder?: string; choices?: string[]; multiline?: boolean }
export interface ActionPreset { uses: string; inputs: ActionInput[]; outputs?: string[] }
export const actionShells = ["bash", "sh", "pwsh", "powershell", "cmd", "python"] as const;
const field = (key: string, placeholder?: string): ActionInput => ({ key, ...(placeholder === undefined ? {} : { placeholder }) });
const boolean = (key: string): ActionInput => ({ key, choices: ["true", "false"] });
export const actionPresets: ActionPreset[] = [
  { uses: "actions/setup-node@v4", inputs: [field("node-version", "22 / lts/* / ${{ matrix.node }}"), field("node-version-file", ".nvmrc"), { key: "cache", choices: ["npm", "yarn", "pnpm"] }, field("cache-dependency-path", "package-lock.json"), field("registry-url", "https://registry.npmjs.org"), field("scope", "@organization"), boolean("check-latest")] },
  { uses: "actions/checkout@v4", inputs: [field("repository", "owner/repository"), field("ref", "main / ${{ github.ref }}"), field("path", "source"), field("fetch-depth", "1 (0 = all history)"), boolean("fetch-tags"), boolean("lfs"), { key: "submodules", choices: ["true", "false", "recursive"] }, boolean("persist-credentials")] },
  { uses: "actions/cache@v4", inputs: [{ key: "path", multiline: true }, field("key", "${{ runner.os }}-${{ hashFiles('**/package-lock.json') }}"), { key: "restore-keys", multiline: true }, boolean("fail-on-cache-miss"), boolean("lookup-only")] },
  { uses: "actions/upload-artifact@v4", inputs: [field("name", "build-output"), { key: "path", placeholder: "dist/**", multiline: true }, { key: "if-no-files-found", choices: ["warn", "error", "ignore"] }, field("retention-days", "14"), field("compression-level", "6"), boolean("overwrite"), boolean("include-hidden-files")] },
  { uses: "actions/download-artifact@v4", inputs: [field("name", "build-output"), field("path", "dist"), field("pattern", "build-*"), boolean("merge-multiple"), field("run-id", "${{ github.run_id }}")] },
  { uses: "actions/setup-python@v5", inputs: [field("python-version", "3.12 / ${{ matrix.python }}"), field("python-version-file", ".python-version"), { key: "cache", choices: ["pip", "pipenv", "poetry"] }, field("cache-dependency-path", "requirements.txt"), boolean("check-latest"), boolean("allow-prereleases")] },
  { uses: "actions/setup-java@v4", inputs: [field("java-version", "21"), { key: "distribution", choices: ["temurin", "zulu", "corretto", "oracle", "microsoft", "semeru"] }, { key: "cache", choices: ["maven", "gradle", "sbt"] }, field("cache-dependency-path", "pom.xml"), boolean("check-latest")] },
  { uses: "docker/login-action@v3", inputs: [field("registry", "ghcr.io"), field("username", "${{ github.actor }}"), field("password", "${{ secrets.REGISTRY_TOKEN }}"), boolean("logout")] },
  { uses: "docker/build-push-action@v6", inputs: [field("context", "."), field("file", "Dockerfile"), { key: "tags", placeholder: "ghcr.io/owner/image:latest", multiline: true }, { key: "platforms", placeholder: "linux/amd64,linux/arm64" }, boolean("push"), boolean("load"), boolean("no-cache"), { key: "build-args", multiline: true }, { key: "cache-from", placeholder: "type=gha", multiline: true }, { key: "cache-to", placeholder: "type=gha,mode=max", multiline: true }] },
];

const actionOutputs: Record<string, string[]> = {
  "actions/setup-node@v4": ["node-version", "cache-hit"],
  "actions/checkout@v4": ["ref", "commit"],
  "actions/cache@v4": ["cache-hit"],
  "actions/upload-artifact@v4": ["artifact-id", "artifact-url", "artifact-digest"],
  "actions/download-artifact@v4": ["download-path"],
  "actions/setup-python@v5": ["python-version", "python-path", "cache-hit"],
  "actions/setup-java@v4": ["distribution", "version", "path", "cache-hit"],
  "docker/build-push-action@v6": ["imageid", "digest", "metadata"],
};
for (const preset of actionPresets) {
  const outputs = actionOutputs[preset.uses];
  if (outputs) preset.outputs = outputs;
}

/** Custom refs may use a different interface. Preserve them as generic input rows. */
export function actionPreset(reference: string): ActionPreset | undefined {
  return actionPresets.find(preset => preset.uses.toLowerCase() === reference.toLowerCase());
}
