import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect,it,vi } from "vitest";
import { selectDisplay } from "../../shared/forms/auto-form";
import { hostRepositoryText,i18n } from "../../shared/i18n/i18n";
import messages from "../../shared/i18n/locales/host-repository-domain.json";
import type { FormConfig } from "../../shared/resources/types";
import { RepositoryMonitor,repositoryEventForm,repositoryEventList } from "./RepositoryMonitor";
const capture=vi.hoisted(()=>({config:undefined as FormConfig<Record<string,unknown>>|undefined}));
vi.mock('../resources/ResourceForm',()=>({ResourceForm:({config}:{config:FormConfig<Record<string,unknown>>})=>{capture.config=config;return null;}}));

it("retains unknown saved subscription events when opening the edit form",async()=>{
  Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
  const host=document.createElement('div'),root=createRoot(host);
  try{
    await act(async()=>root.render(<RepositoryMonitor workspace="ops" connectionId="c1" permissions={new Set()}/>));
    const initial=capture.config!.editValues!;
    expect(initial({provider:'git',events:['push','future.event','merge']})).toMatchObject({events:['push','future.event']});
    expect(initial({provider:'codeup',events:['push','repository.future','repository.created']})).toMatchObject({events:['push','repository.future']});
  }finally{await act(async()=>root.unmount());}
});
it("migrates legacy webhook URLs and languages into editable channels without losing configuration",async()=>{
  Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
  const host=document.createElement('div'),root=createRoot(host);
  try{
    await act(async()=>root.render(<RepositoryMonitor workspace="ops" connectionId="c1" permissions={new Set()}/>));
    const config=capture.config!;
    for(const provider of ['git','codeup']){
      const row={provider,webhook:'https://notify.example.invalid/token',notification_language:'ja-JP'};
      expect(config.editValues!(row)).toMatchObject({webhook:row.webhook,notification_language:'ja-JP'});
      const fields=config.sections.flatMap(section=>section.fields);
      expect(config.editValues!(row).notification_channels).toEqual([{name:'Webhook',webhook:row.webhook,language:'ja-JP',enabled:true}]);
      expect(fields.find(field=>field.key==='notification_channels')!.type).toBe('custom');
      expect(fields.some(field=>field.key==='webhook')).toBe(false);
    }
    expect(config.editValues!({provider:'codeup'}).notification_language).toBe('en-US');
  }finally{await act(async()=>root.unmount());}
});
it("updates subscription labels and event options immediately in all six languages",async()=>{
  Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
  const host=document.createElement('div'),root=createRoot(host);
  try{
    await act(async()=>root.render(<RepositoryMonitor workspace="ops" connectionId="c1" permissions={new Set()}/>));
    for(const locale of ['en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY'] as const){
      await act(async()=>{await i18n.changeLanguage(locale);});
      expect(capture.config!.sections[0]!.title).toBe(messages[locale]['repository-monitor.056']);
      const event=repositoryEventList('ops').columns.find(column=>column.key==='event')!;
      expect(event.render!({event:'push'})).toBe(messages[locale]['repository-monitor.005']);
      expect(host.textContent).toContain(hostRepositoryText('repository-monitor.089'));
    }
  }finally{await act(async()=>root.unmount());await i18n.changeLanguage('en-US');}
});

it("renders absent event dates as empty and preserves valid zero counts", () => {
  const fields=repositoryEventForm().sections.flatMap(section=>section.fields);
  const date=fields.find(field=>field.key==='created_at')!;
  for(const value of [0,'0',null,undefined,'']) expect(date.display!({created_at:value})).toBe('—');
  expect(fields.find(field=>field.key==='total_commits')!.when!({total_commits:0})).toBe(true);
  const columns=repositoryEventList('ops').columns;
  expect(columns.find(column=>column.key==='created_at')!.render!({created_at:0})).toBe('—');
  expect(columns.find(column=>column.key==='event')!.render!({event:'future.event'})).toBe(selectDisplay('future.event',{options:[]}));
});

it("shows every subscription event with its selected or unselected read-only checkbox", async () => {
  Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
  const host=document.createElement('div'),root=createRoot(host);
  try {
    await act(async()=>root.render(<RepositoryMonitor workspace="ops" connectionId="c1" permissions={new Set()} />));
    const field=capture.config!.sections.flatMap(section=>section.fields).find(field=>field.key==='events')!;
    const render=(provider: string, events: string[]) => field.render!({workspace:"ops",draft:{provider,events},readOnly:true,onPatch:vi.fn(),onValidityChange:vi.fn()});
    await act(async()=>root.render(<>{render('github',['push','tag.created'])}</>));
    let boxes=[...host.querySelectorAll<HTMLButtonElement>('[role="checkbox"]')];
    expect(boxes).toHaveLength(9);expect(boxes.filter(box=>box.getAttribute('aria-checked')==='true')).toHaveLength(2);expect(boxes.every(box=>box.disabled)).toBe(true);
    await act(async()=>root.render(<>{render('git',[])}</>));
    boxes=[...host.querySelectorAll<HTMLButtonElement>('[role="checkbox"]')];
    expect(boxes).toHaveLength(6);expect(boxes.every(box=>box.getAttribute('aria-checked')==='false')).toBe(true);
  } finally {await act(async()=>root.unmount());}
});
