import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { i18n, hostRepositoryText } from "../i18n";
import { HostWorkspace } from "./HostWorkspace";
vi.mock('./HostFiles',()=>({HostFiles:()=>null}));
vi.mock('./HostTerminal',()=>({HostTerminal:()=>null}));
vi.mock('./HostLatency',()=>({HostLatency:()=>null}));
vi.mock('./HostTelemetry',()=>({HostTelemetry:({children}:{children:React.ReactNode})=>children,TerminalMetrics:()=>null}));
it("translates existing workspace tabs without renaming the host or recreating sessions",async()=>{
 Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
 vi.stubGlobal('fetch',vi.fn(async()=>({ok:true,status:200,json:async()=>({name:'Production 主机'})})));
 const host=document.createElement('div'),root=createRoot(host);
 try{
  await act(async()=>root.render(<HostWorkspace workspace="ops" hostId="1" permissions={new Set(['hosts.files'])}/>));
  for(const locale of ['en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY']){
   await act(async()=>{await i18n.changeLanguage(locale);});
   expect(host.querySelector('h1')?.textContent).toBe('Production 主机');
   expect(host.querySelector('[role="tablist"][aria-label="'+hostRepositoryText('host.084')+'"]')?.textContent).toContain(hostRepositoryText('host.072')+' 1');
  }
  expect(fetch).toHaveBeenCalledTimes(1);
 }finally{await act(async()=>root.unmount());vi.unstubAllGlobals();await i18n.changeLanguage('en-US');}
});
