import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { FileActions, fileSource, type FileRow } from "./PlatformFileCatalog";
const base: FileRow = {id:"11",workspace_id:"22",workspace:"Test",name:"test.json",sha256:"a".repeat(64),mime:"application/json",size:12,created_at:0,kind:"asset",workflow_id:"33",run_id:"44"};
it("links each known origin to a concrete record in a separate tab", () => {
  expect(fileSource(base)).toMatch(/workspaces\/22\/workflows\/33\?execution=44$/);
  expect(fileSource({...base,kind:"artifact"})).toMatch(/workspaces\/22\/pipeline-runs\/44$/);
  const host=document.createElement("div"); host.innerHTML=renderToStaticMarkup(<FileActions row={base}/>);
  const source=host.querySelector('a[target="_blank"]');
  expect(source?.getAttribute("rel")).toContain("noopener");
  expect(source?.getAttribute("href")).toContain("execution=44");
  expect(host.querySelector('a[download]')?.getAttribute("href")).toBe("/api/platform/monitor/files/catalog/11/content");
});
it("never fabricates a dashboard or an execution route when origin is absent", () => {
  const orphan={...base,workflow_id:null}; expect(fileSource(orphan)).toBeNull();
  const host=document.createElement("div"); host.innerHTML=renderToStaticMarkup(<FileActions row={orphan}/>);
  expect(host.querySelector('a[target="_blank"]')).toBeNull();
});
