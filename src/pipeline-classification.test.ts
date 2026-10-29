import { expect, it } from "vitest";
import { formConfigs, listConfigs } from "./resources";
import { nav } from "./nav";
it("pipeline classification uses shared resource forms and filters", () => {
 const form=formConfigs("ops").pipelines!;
 const keys=form.sections.flatMap(section=>section.fields.map(field=>field.key));
 expect(keys).toEqual(expect.arrayContaining(["group_id","tag_ids"]));
 expect(keys).not.toContain("application_id");
 expect(keys).not.toContain("description");
 expect(keys).not.toContain("environment_ids");
 const filters=listConfigs("ops").pipelines!.filters!.map(filter=>filter.key);
 expect(filters).toEqual(expect.arrayContaining(["group_id","application_id","tag_id"]));
 expect(filters).not.toContain("environment_id");
 expect(formConfigs("ops")["pipeline-groups"]).toBeDefined();
 expect(listConfigs("ops")["pipeline-groups"]).toBeDefined();
 expect(nav.find(group=>group.key==="build")!.items.find(item=>item.resource==="pipeline-groups")?.parent).toBe("pipelines");
});
it("existing pipeline editors can edit without fetching newly introduced classification permissions", () => {
 const fields=formConfigs("ops",new Set(["pipelines.read","pipelines.update","hosts.read","host-groups.read","repositories.read"])).pipelines!.sections.flatMap(section=>section.fields);
 const visible=fields.filter(field=>!field.when||field.when({}));
 expect(visible.map(field=>field.key)).not.toEqual(expect.arrayContaining(["group_id"]));
 for(const key of ["group_id","application_id","tag_ids","environment_ids"])expect(visible.some(field=>field.key===key)).toBe(false);
});
it("Actions executor options do not request inaccessible host groups", async () => {
 const calls:string[]=[];
 const previous=globalThis.fetch;
 globalThis.fetch=async(input)=>{calls.push(String(input));return new Response(JSON.stringify({items:[],total:0,page:1,page_size:200}),{headers:{"Content-Type":"application/json"}})};
 try {
  const editor=formConfigs("ops",new Set(["hosts.read"])).pipelines!.sections.flatMap(section=>section.fields).find(field=>field.type==="actions-editor")!;
  await (editor.options as ()=>Promise<unknown>)();
  expect(calls.some(url=>url.includes('/hosts'))).toBe(true);
  expect(calls.some(url=>url.includes('/host-groups'))).toBe(false);
 } finally {globalThis.fetch=previous;}
});
