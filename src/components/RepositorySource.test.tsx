import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { RepositorySource } from "./RepositorySource";

let root: Root, host: HTMLDivElement;
let patch = vi.fn<(value: Record<string, unknown>) => void>();
let validity = vi.fn<(value: boolean) => void>();
let requests: string[];
const credentials = [{id:"11",name:"Team PAT",kind:"token"},{id:"12",name:"Deploy key",kind:"ssh-key"},{id:"13",name:"Git password",kind:"password"}];
const select = async (label: string, value: string) => { const node = host.querySelector<HTMLSelectElement>(`[aria-label="${label}"]`)!; expect(node).not.toBeNull(); await act(async()=>{node.value=value;node.dispatchEvent(new Event("change",{bubbles:true}));}); };
async function mount(draft: Record<string, unknown> = {name:"Team connection",provider:"github",credential_id:"0"}, readOnly=false) {
 function Harness(){const [state,setState]=useState(draft);return <RepositorySource workspace="ops" draft={state} readOnly={readOnly} onChange={value=>{patch(value);setState(current=>({...current,...value}));}} onValidityChange={validity}/>;}
 await act(async()=>root.render(<Harness/>));
}
beforeEach(()=>{
 Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});history.replaceState({},"","/ops/build/repositories/new");
 host=document.createElement("div");document.body.append(host);root=createRoot(host);patch=vi.fn();validity=vi.fn();requests=[];
 vi.stubGlobal("fetch",vi.fn(async(url:string)=>{requests.push(url);return {ok:true,status:200,json:async()=>({items:url.includes("/credentials")?credentials:[{id:"github",web_available:true}]})};}));
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();vi.restoreAllMocks();});
it("named platform connections select credential IDs without repository, branch or secret inputs",async()=>{
 await mount();
 expect(requests.some(url=>url.includes("/credentials"))).toBe(true);
 await select("授权认证","11");
 expect(patch).toHaveBeenLastCalledWith({credential_id:"11"});
 expect(validity).toHaveBeenLastCalledWith(false);
 expect(host.querySelector('input[type="password"]')).toBeNull();
 expect(host.textContent).not.toContain("选择平台仓库");
 expect(host.textContent).not.toContain("默认分支");
 expect(host.querySelector('[aria-label="授权认证"]')?.textContent).not.toContain("Deploy key");
 expect(host.querySelector('a[target="_blank"]')?.getAttribute("href")).toBe("/ops/build/credentials/new");
});
it("ordinary Git permits an optional URL and all existing Git credential kinds",async()=>{
 await mount({name:"Git connection",provider:"git",credential_id:"0"});
 expect(validity).toHaveBeenLastCalledWith(false);
 expect(host.querySelector<HTMLInputElement>('[aria-label="Git 仓库地址（可选）"]')?.required).toBe(false);
 expect(host.querySelector('[aria-label="授权认证"]')?.textContent).toContain("Deploy key");
 await select("授权认证","12");expect(patch).toHaveBeenLastCalledWith({credential_id:"12"});
});
it("credential-list denial is visible and retains the saved credential without copying its secret",async()=>{
 vi.stubGlobal("fetch",vi.fn(async(url:string)=>({ok:!url.includes("/credentials"),status:url.includes("/credentials")?403:200,json:async()=>url.includes("/credentials")?{error:"无权查看授权认证"}:{items:[]}})));
 await mount({id:"saved",name:"Saved",provider:"github",credential_id:"88"});
 expect(host.querySelector('[role="alert"]')?.textContent).toContain("无权查看授权认证");
 expect(host.querySelector<HTMLSelectElement>('[aria-label="授权认证"]')?.value).toBe("88");
 expect(patch).not.toHaveBeenCalled();expect(validity).toHaveBeenLastCalledWith(false);
});
it("empty credential results explain the next action and block incomplete platform connections",async()=>{
 vi.stubGlobal("fetch",vi.fn(async()=>({ok:true,status:200,json:async()=>({items:[]})})));
 await mount();expect(host.textContent).toContain("没有可用的授权认证");expect(host.textContent).toContain("新建授权认证");expect(validity).toHaveBeenLastCalledWith(true);
});
it("read-only connections show configuration without fetching credentials or asking for a repository",async()=>{
 await mount({provider:"gitea",credential_id:"11",base_url:"https://git.example.invalid",api_url:"https://git.example.invalid/api/v1"},true);
 expect(requests).toEqual([]);expect(host.textContent).toContain("https://git.example.invalid");expect(host.querySelector("select")).toBeNull();
});
it("OAuth validates the popup sender and navigates directly to the created connection",async()=>{
 const opened={closed:false,close:vi.fn(),location:{href:"about:blank"}};
 vi.spyOn(window,"open").mockReturnValue(opened as unknown as Window);
 vi.stubGlobal("BroadcastChannel",class {onmessage=null;close(){} });
 vi.stubGlobal("fetch",vi.fn(async(url:string)=>{requests.push(url);return {ok:true,status:200,json:async()=>url.includes("/actions/authorize")?{url:"https://github.example.invalid/oauth"}:{items:url.includes("/credentials")?credentials:[{id:"github",web_available:true}]}};}));
 await mount();await select("平台授权方式","web");
 await act(async()=>[...host.querySelectorAll("button")].find(button=>button.textContent==="打开网页授权")!.click());
 const url=new URL(requests.find(url=>url.includes("/actions/authorize"))!,location.origin);
 expect(url.searchParams.get("name")).toBe("Team connection");expect(url.searchParams.get("channel")).toBeTruthy();
 await act(async()=>window.dispatchEvent(new MessageEvent("message",{origin:"https://attacker.invalid",source:opened as unknown as Window,data:{type:"repository-authorized",provider:"github",status:"connected",connection_id:"45"}})));
 expect(location.pathname).toBe("/ops/build/repositories/new");
 await act(async()=>window.dispatchEvent(new MessageEvent("message",{origin:location.origin,source:opened as unknown as Window,data:{type:"repository-authorized",provider:"github",status:"connected",connection_id:"45"}})));
 expect(location.pathname).toBe("/ops/build/repositories/45");expect(patch).not.toHaveBeenCalled();
 expect(requests.some(url=>url.includes("/repositories?"))).toBe(false);
});
it("credential loading is visible and resolves into the existing picker",async()=>{
 let resolveCredentials!: (value: unknown) => void;
 vi.stubGlobal("fetch",vi.fn(async(url:string)=>url.includes("/credentials")?new Promise(resolve=>{resolveCredentials=resolve;}):{ok:true,status:200,json:async()=>({items:[]})}));
 await mount();expect(host.textContent).toContain("正在读取授权认证");expect(host.querySelector<HTMLSelectElement>('[aria-label="授权认证"]')?.disabled).toBe(true);
 await act(async()=>resolveCredentials({ok:true,status:200,json:async()=>({items:credentials})}));
 expect(host.querySelector<HTMLSelectElement>('[aria-label="授权认证"]')?.disabled).toBe(false);expect(host.textContent).toContain("Team PAT");
});
it("reauthorizing a saved OAuth connection sends its ID without clearing the existing binding",async()=>{
 const opened={closed:false,close:vi.fn(),location:{href:"about:blank"}};vi.spyOn(window,"open").mockReturnValue(opened as unknown as Window);
 vi.stubGlobal("BroadcastChannel",class {onmessage=null;close(){} });
 vi.stubGlobal("fetch",vi.fn(async(url:string)=>{requests.push(url);return {ok:true,status:200,json:async()=>url.includes("/actions/authorize")?{url:"https://github.example.invalid/oauth"}:{items:url.includes("/credentials")?credentials:[{id:"github",web_available:true}]}};}));
 await mount({id:"44",name:"Saved OAuth",provider:"github",credential_id:"11",authorization:"oauth"});
 expect(validity).toHaveBeenLastCalledWith(false);
 await act(async()=>[...host.querySelectorAll("button")].find(button=>button.textContent==="重新网页授权")!.click());
 expect(new URL(requests.find(url=>url.includes("/actions/authorize"))!,location.origin).searchParams.get("connection_id")).toBe("44");
 expect(patch).not.toHaveBeenCalled();
 await act(async()=>[...host.querySelectorAll("button")].find(button=>button.textContent==="取消授权")!.click());
 expect(validity).toHaveBeenLastCalledWith(false);expect(opened.close).toHaveBeenCalled();
});
