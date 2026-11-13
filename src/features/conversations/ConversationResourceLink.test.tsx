import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { ConversationResourceLink, conversationResource } from "./ConversationResourceLink";
const { request }=vi.hoisted(()=>({request:vi.fn()}));
vi.mock("../../shared/api/api",()=>({api:request,errorText:(e:Error)=>e.message}));
vi.mock("../../shared/i18n/locale",()=>({currentLocale:()=> 'zh-CN',localizedPath:(path:string)=>'/zh-CN'+path}));
Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
afterEach(()=>vi.resetAllMocks());
it("maps only same-origin, same-workspace workflow detail endpoints",()=>{
  const endpoint='/workspaces/1/conversations/2';
  expect(conversationResource('/api/workspaces/1/workflows/123',endpoint)).toEqual({workspace:'1',kind:'workflows',id:'123'});
  for(const url of ['/api/workspaces/2/workflows/123','https://foreign.invalid/api/workspaces/1/workflows/123','/api/workspaces/1/workflows/123/executions','/api/workspaces/1/workflows/123?download=true'])expect(conversationResource(url,endpoint)).toBeNull();
});
it("resolves the actual authorized parent and never navigates to the JSON API",async()=>{
  request.mockResolvedValue({workflow_id:'456'});const host=document.createElement('div'),root=createRoot(host);
  try{
    await act(()=>root.render(<ConversationResourceLink resource={{workspace:'1',kind:'workflow-executions',id:'123'}}>Execution</ConversationResourceLink>));
    expect(host.querySelector('a')?.getAttribute('href')).toBe('/zh-CN/workspaces/1/workflows/456?execution=123');
    expect(host.querySelector('a')?.getAttribute('target')).toBe('_blank');
  }finally{await act(()=>root.unmount());}
});
it("recognizes pipeline resources but keeps unsupported and cross-workspace URLs untouched",()=>{
  const endpoint='/workspaces/1/conversations/2';
  for (const kind of ['pipelines','pipeline-runs']) {
    expect(conversationResource(`/api/workspaces/1/${kind}/123`,endpoint)).toEqual({workspace:'1',kind,id:'123'});
  }
  expect(conversationResource('/api/workspaces/1/pipeline-artifacts/123/content',endpoint)).toEqual({workspace:'1',kind:'pipeline-artifacts',id:'123'});
  for (const url of ['/api/workspaces/2/pipeline-runs/123','/api/workspaces/1/pipeline-artifacts/123','/api/workspaces/1/pipelines/123/content','/api/workspaces/1/pipeline-runs/123/logs','/api/workspaces/1/pipelines/123?edit=true','https://foreign.invalid/api/workspaces/1/pipelines/123']) {
    expect(conversationResource(url,endpoint)).toBeNull();
  }
});
it.each(['pipelines','pipeline-runs'])("opens %s in its localized existing page without requesting JSON",async kind=>{
  const host=document.createElement('div'),root=createRoot(host);
  try {
    await act(()=>root.render(<ConversationResourceLink resource={{workspace:'1',kind,id:'123'}}>Open</ConversationResourceLink>));
    expect(host.querySelector('a')?.getAttribute('href')).toBe(`/zh-CN/workspaces/1/${kind}/123`);
    expect(host.querySelector('a')?.getAttribute('target')).toBe('_blank');
    expect(request).not.toHaveBeenCalled();
  } finally {await act(()=>root.unmount());}
});
it("downloads artifact content through the existing authorized endpoint",async()=>{
  const host=document.createElement('div'),root=createRoot(host);
  try {
    await act(()=>root.render(<ConversationResourceLink resource={{workspace:'1',kind:'pipeline-artifacts',id:'123'}}>receipt.zip</ConversationResourceLink>));
    const link=host.querySelector('a')!;
    expect(link.getAttribute('href')).toBe('/api/workspaces/1/pipeline-artifacts/123/content');
    expect(link.hasAttribute('download')).toBe(true);
    expect(link.hasAttribute('target')).toBe(false);
    expect(request).not.toHaveBeenCalled();
  } finally {await act(()=>root.unmount());}
});
it("opens the artifact list with the exact run filter returned by the Agent",async()=>{
  const endpoint='/workspaces/1/conversations/2';
  const resource=conversationResource('/api/workspaces/1/pipeline-artifacts?run_id=123',endpoint);
  expect(resource).toEqual({workspace:'1',kind:'pipeline-artifacts-list',id:'123'});
  for (const url of ['/api/workspaces/2/pipeline-artifacts?run_id=123','/api/workspaces/1/pipeline-artifacts?run_id=123&run_id=456','/api/workspaces/1/pipeline-artifacts?run_id=123&page=1','/api/workspaces/1/pipeline-artifacts?run_id=abc','/api/workspaces/1/pipeline-artifacts?pipeline_id=123','https://foreign.invalid/api/workspaces/1/pipeline-artifacts?run_id=123']) {
    expect(conversationResource(url,endpoint)).toBeNull();
  }
  const host=document.createElement('div'),root=createRoot(host);
  try {
    await act(()=>root.render(<ConversationResourceLink resource={resource!}>Artifacts</ConversationResourceLink>));
    expect(host.querySelector('a')?.getAttribute('href')).toBe('/zh-CN/workspaces/1/pipeline-artifacts?run_id=123');
    expect(request).not.toHaveBeenCalled();
  }finally{await act(()=>root.unmount());}
});
it("preserves denial as an error and allows retry without exposing an API link",async()=>{
  request.mockRejectedValueOnce(new Error('Access denied')).mockResolvedValueOnce({workflow_id:'456'});
  const host=document.createElement('div'),root=createRoot(host);
  try{
    await act(()=>root.render(<ConversationResourceLink resource={{workspace:'1',kind:'workflow-executions',id:'123'}}>Execution</ConversationResourceLink>));
    expect(host.querySelector('a')).toBeNull();expect(host.querySelector('[role=alert]')?.textContent).toBe('Access denied');
    await act(()=>host.querySelector('button')!.click());expect(host.querySelector('a')?.getAttribute('href')).toContain('execution=123');
  }finally{await act(()=>root.unmount());}
});
