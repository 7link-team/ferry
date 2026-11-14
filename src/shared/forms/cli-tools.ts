/** Display names of the CLIs a Runner can drive; ids match ferry_protocol::agent::Runtime. */
export const CLI_TOOL_LABELS: Record<string, string> = {
  claude: "Claude Code",
  codex: "Codex",
  gemini: "Gemini CLI",
  qwen: "Qwen Code",
  opencode: "OpenCode",
  pi: "Pi",
  kimi: "Kimi Code",
  grok: "Grok Build",
  dsh: "DeepSeek Harness",
  copilot: "GitHub Copilot CLI",
  codebuddy: "CodeBuddy Code",
};
export const cliToolLabel = (runtime: string) => CLI_TOOL_LABELS[runtime] ?? runtime;
