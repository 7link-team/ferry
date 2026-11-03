import { expect, it } from "vitest";
import { deploymentsForms, deploymentsLists } from "./resource-config";
import { pipelinesForms } from "../pipelines/resource-config";

it("connects optional application categories and environment name suggestions to safe workspace preferences", async () => {
  const context = { workspace: "ops", navigate: () => {}, dictionaries: { "app-category": [{ value: "web", label: "Web service" }] } };
  const list = deploymentsLists(context).deployments!;
  const filter = list.filters!.find(filter => filter.key === "category")!;
  expect(filter.options).toEqual(context.dictionaries["app-category"]);
  const column = list.columns.find(column => column.key === "category")!;
  expect(column.render!({ category: "web" })).toBe("Web service");
  expect(column.render!({ category: "removed-category" })).toBe("removed-category");
  expect(column.render!({ category: "" })).toBe("—");
  const form = deploymentsForms(context).deployments!;
  expect(form.blank.category).toBe("");
  const field = form.sections.flatMap(section => section.fields).find(field => field.key === "category")!;
  expect(field.type).toBe("select");
  expect(field.required).not.toBe(true);
  const environment = pipelinesForms(context)["pipeline-environments"]!.sections.flatMap(section => section.fields).find(field => field.key === "name")!;
  expect(environment.type).toBe("text");
  expect(typeof environment.options).toBe("function");
});
