import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import type { Option } from "../../shared/resources/types";
import { CONNECTIONS, hostsLists, type Host } from "./resource-config";
import { label } from "../../shared/resources/config";
import { hostsForms } from "./resource-config";

it("excludes the edited host from jump choices while allowing other hosts and new records", () => {
  const config = hostsForms({ workspace: "ops", navigate: () => {} }).hosts!;
  const jump = config.sections.flatMap(section => section.fields).find(field => field.key === "jump_host_id")!;
  expect(jump.optionFilter!({ value: "17", label: "Self" }, { id: "17" })).toBe(false);
  expect(jump.optionFilter!({ value: "18", label: "Jump" }, { id: "17" })).toBe(true);
  expect(jump.optionFilter!({ value: "17", label: "Jump" }, {})).toBe(true);
});

it("renders shared execution status and small HostAgent versions without per-row requests", () => {
  const config = hostsLists({ workspace: "ops", navigate: () => {}, hostStates: [] }).hosts!;
  const row = { connection: "runner", agent_version: "0.1.0", execution: { enabled: true, shared: true, capacity: 2 }, execution_status: { state: "available", active: 0, capacity: 2, running_jobs: [] } } as unknown as Host;
  const connection = renderToStaticMarkup(<>{config.columns.find(column => column.key === "connection")!.render!(row)}</>);
  expect(connection).toContain("HostAgent 0.1.0");
  expect(connection).toContain("text-xs");
  const capacity = renderToStaticMarkup(<>{config.columns.find(column => column.key === "execution_status")!.render!(row)}</>);
  expect(capacity).toContain(i18n.t("hostExecution.sharedBadge", { ns: "execution-domain" }));
  const ssh = renderToStaticMarkup(<>{config.columns.find(column => column.key === "connection")!.render!({ ...row, connection: "ssh" })}</>);
  expect(ssh).not.toContain("HostAgent 0.1.0");
});

it.each([
  ["en-US", "Agent (HostAgent)"],
  ["zh-CN", "主机代理（HostAgent）"],
  ["zh-TW", "主機代理（HostAgent）"],
  ["ja-JP", "エージェント（HostAgent）"],
  ["ko-KR", "에이전트(HostAgent)"],
  ["ms-MY", "Ejen (HostAgent)"],
])("keeps host connection labels distinct from pipeline runners in %s", async (locale, expected) => {
  await i18n.changeLanguage(locale);
  expect(label(CONNECTIONS, "runner")).toBe(expected);
  const config = hostsLists({ workspace: "ops", navigate: () => {}, hostStates: [] }).hosts!;
  const connection = config.columns.find(column => column.key === "connection")!;
  expect(renderToStaticMarkup(<>{connection.render!({ connection: "runner" } as Host)}</>)).toContain(expected);
  expect(label(CONNECTIONS, "ssh")).toContain("SSH");
});

it("uses configured state colors in host rows and filters while preserving labels and fallback states", async () => {
  await i18n.changeLanguage("en-US");
  const hostStates: Option[] = [
    { value: "online", label: "Connected", color: "#123456" },
    { value: "offline", label: "Offline", color: "#ffffff" },
    { value: "unknown", label: "Unknown" },
  ];
  const config = hostsLists({ workspace: "ops", navigate: () => {}, hostStates }).hosts!;
  const state = config.columns.find(column => column.key === "state")!;
  const online = renderToStaticMarkup(<>{state.render!({ state: "online", connection: "ssh" } as Host)}</>);
  expect(online).toContain("background-color:#123456;color:#ffffff");
  expect(online).toContain("Connected");
  const offline = renderToStaticMarkup(<>{state.render!({ state: "offline", connection: "runner" } as Host)}</>);
  expect(offline).toContain("background-color:#ffffff;color:#000000");
  expect(offline).toContain("Disconnected");
  const unknown = renderToStaticMarkup(<>{state.render!({ state: "unknown", connection: "ssh" } as Host)}</>);
  expect(unknown).toContain("Unknown");
  expect(unknown).not.toContain("background-color");
  const options = config.filters!.find(filter => filter.key === "state")!.options as Option[];
  expect(options.map(option => option.value)).toEqual(["online", "offline", "unknown"]);
  expect(renderToStaticMarkup(<>{options[0]!.content}</>)).toContain("background-color:#123456");
});
