import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ProviderFields, type Provider } from "./ai-provider-fields";
import { commonText } from "../i18n/i18n";

it("leaves provider quotas to platform policy while preserving legacy values on edit",async()=>{
 Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const draft:Provider={id:"1",protocol:"openai",name:"Example",credential_id:"2",base_url:"https://provider.example.invalid",qps:1,concurrency:1,version:1};const patch=vi.fn();
 try {await act(async()=>root.render(<ProviderFields draft={draft} onChange={patch}/>));expect(host.querySelectorAll('input[type="number"]')).toHaveLength(0);
 const input=host.querySelector<HTMLInputElement>(`input[aria-label="${commonText("common.name")}"]`)!;
 await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,"Updated");input.dispatchEvent(new Event("input",{bubbles:true}));});
 expect(patch).toHaveBeenLastCalledWith({...draft,name:"Updated"});
 await act(async()=>root.render(<ProviderFields draft={draft} readOnly onChange={patch}/>));expect(host.querySelectorAll("dt")).toHaveLength(2);
 }finally{await act(async()=>root.unmount());host.remove();}
});
