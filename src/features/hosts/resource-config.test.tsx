import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import type { Option } from "../../shared/resources/types";
import { hostsLists, type Host } from "./resource-config";

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
