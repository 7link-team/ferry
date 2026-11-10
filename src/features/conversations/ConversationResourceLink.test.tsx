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
it("preserves denial as an error and allows retry without exposing an API link",async()=>{
  request.mockRejectedValueOnce(new Error('Access denied')).mockResolvedValueOnce({workflow_id:'456'});
  const host=document.createElement('div'),root=createRoot(host);
  try{
    await act(()=>root.render(<ConversationResourceLink resource={{workspace:'1',kind:'workflow-executions',id:'123'}}>Execution</ConversationResourceLink>));
    expect(host.querySelector('a')).toBeNull();expect(host.querySelector('[role=alert]')?.textContent).toBe('Access denied');
    await act(()=>host.querySelector('button')!.click());expect(host.querySelector('a')?.getAttribute('href')).toContain('execution=123');
  }finally{await act(()=>root.unmount());}
});
