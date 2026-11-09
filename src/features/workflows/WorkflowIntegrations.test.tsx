import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../shared/api/api";
import { i18n, workflowText as t, hostRepositoryText, commonText } from "../../shared/i18n/i18n";
import { WorkflowAssetContext } from "./WorkflowAssets";
import { outputPortLabel, WorkflowNodePicker } from "./WorkflowNodePicker";
import { WorkflowNodeParameters } from "./WorkflowNodeParameters";
import { emptyGraph, newNode } from "./graph";
import {
  integrationManifests,
  integrationText,
  integrationTitle,
  installIntegrationCatalog,
} from "./integrations";

vi.mock("../../shared/api/api", () => ({
  api: vi.fn(),
  errorText: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));
let root: Root, host: HTMLDivElement;
const calls = vi.mocked(api);
beforeEach(async () => {
  Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
  await i18n.changeLanguage("zh-CN");
  calls.mockReset();
  calls.mockResolvedValue({
    items: [
      { id: "7", name: "Slack token", username: "slack", kind: "token" },
      { id: "8", name: "GitHub token", username: "github", kind: "token" },
    ],
    page: 1,
    page_size: 200,
    total: 2,
  });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});
const button = (label: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) =>
      button.getAttribute("aria-label") === label ||
      button.textContent?.trim() === label,
  )!;

it("offers clone and command together in the code and build category", async()=>{
  const select=vi.fn();
  await act(async()=>root.render(<WorkflowNodePicker inserting={false} accepts={()=>true} select={select} close={()=>{}}/>));
  await act(async()=>button(t('category.build')).click());
  const clone=integrationManifests.find(node=>node.git?.operation==='clone' && !node.git.provider)!;
  expect(button(integrationTitle(clone))).toBeDefined();
  expect(button(t('hostCommand'))).toBeDefined();
  await act(async()=>button(t('hostCommand')).click());
  expect(select).toHaveBeenCalledWith('host.command');
  await input(t('search'),'克隆');
  expect(button(integrationTitle(clone))).toBeDefined();
});

it("edits host command without JSON and exposes the supported shell contract",async()=>{
  let current:Record<string,unknown>={};
  function Harness(){const [node,setNode]=useState(()=>newNode(emptyGraph(),'host.command','Build'));current=node.params;return <WorkflowNodeParameters node={node} readOnly={false} invalid={()=>{}} update={params=>setNode(value=>({...value,params}))}/>;}
  await act(async()=>root.render(<Harness/>));
  expect(host.querySelector<HTMLSelectElement>(`select[aria-label="${t('commandShell')}"]`)?.options.length).toBe(2);
  await input(t('commandScript'),'cd /srv/app\nmake test');
  expect(current.command).toBe('cd /srv/app\nmake test');
  expect(host.textContent).toContain('$FERRY_INPUT_MESSAGE');
  await act(async()=>root.render(<WorkflowNodeParameters node={{...newNode(emptyGraph(),'host.command','Build'),params:current}} readOnly invalid={()=>{}} update={()=>{}}/>));
  expect(host.querySelector<HTMLTextAreaElement>('textarea')?.readOnly).toBe(true);
});

it("requires a shared connection for new platform nodes and fills the provider identity", async () => {
  calls.mockImplementation(async (path) => String(path).includes('/repositories?') ? {items:[{name:'app',full_name:'team/app',url:'https://code.example/team/app.git',default_branch:'main',remote_id:'42'}],has_more:false} : String(path).includes('/code-connections') ? {items:[{id:'7',name:'Existing Codeup',provider:'codeup',state:'connected'},{id:'8',name:'GitHub',provider:'github',state:'connected'}],total:2,page_size:200} : {items:[],total:0,page_size:200});
  let current:Record<string,unknown>={};
  function Harness() {
    const [node,setNode]=useState(()=>newNode(emptyGraph(),'integration.codeup.branch.create','Create branch'));
    current=node.params;
    return <WorkflowAssetContext.Provider value={{workspace:'11',workflow:''}}><WorkflowNodeParameters node={node} readOnly={false} invalid={()=>{}} update={params=>setNode(value=>({...value,params}))}/></WorkflowAssetContext.Provider>;
  }
  await act(async()=>root.render(<Harness/>));
  expect(button(t('integration.createCredential'))).toBeUndefined();
  expect(host.querySelector('select[aria-label="仓库连接"]')?.textContent).toContain('Existing Codeup');
  expect(host.querySelector('select[aria-label="仓库连接"]')?.textContent).not.toContain('GitHub');
  await input(t('integration.connection'),'7');
  await input(t('integration.chooseRepository'),'https://code.example/team/app.git');
  expect(current.connection_id).toBe('7');expect(current.repository_id).toBe('42');
  expect(current).not.toHaveProperty('credential_id');expect(current).not.toHaveProperty('organization_id');
  expect(button(t('integration.independent'))).toBeUndefined();
  expect(button(t('integration.createCredential'))).toBeUndefined();
});

it("retains legacy credentials read-only and switches explicitly to a shared connection",async()=>{
  const original=newNode(emptyGraph(),'integration.github.repository.get','Legacy');
  let current:Record<string,unknown>={credential_id:'8',owner:'team',repo:'app'};
  function Harness(){const [params,setParams]=useState(current);current=params;return <WorkflowAssetContext.Provider value={{workspace:'11',workflow:'22'}}><WorkflowNodeParameters node={{...original,params}} readOnly={false} invalid={()=>{}} update={setParams}/></WorkflowAssetContext.Provider>;}
  await act(async()=>root.render(<Harness/>));
  expect(host.textContent).toContain(t('integration.legacyAuth'));
  expect(host.querySelector<HTMLSelectElement>(`[aria-label="${t('integration.credential')}"]`)?.disabled).toBe(true);
  expect(button(t('integration.createCredential'))).toBeUndefined();
  await act(async()=>button(t('integration.useConnection')).click());
  expect(current).not.toHaveProperty('credential_id');
  expect(host.querySelector(`[aria-label="${t('integration.connection')}"]`)).not.toBeNull();
});

it("offers anonymous HTTP Git separately and keeps local Git free of connection fields",async()=>{
  const original=newNode(emptyGraph(),'integration.git.repository.clone','Clone');
  let current:Record<string,unknown>=original.params;
  function Harness(){const [params,setParams]=useState(current);current=params;return <WorkflowAssetContext.Provider value={{workspace:'11',workflow:''}}><WorkflowNodeParameters node={{...original,params}} readOnly={false} invalid={()=>{}} update={setParams}/></WorkflowAssetContext.Provider>;}
  await act(async()=>root.render(<Harness/>));
  await input(t('integration.gitAuthentication'),'anonymous');
  expect(current.auth_mode).toBe('anonymous');expect(current).not.toHaveProperty('connection_id');
  expect(host.querySelector(`[aria-label="${t('integration.connection')}"]`)).toBeNull();
  await input(t('integration.gitAuthentication'),'connection');
  expect(current.auth_mode).toBe('connection');
  await act(async()=>root.render(<WorkflowNodeParameters node={newNode(emptyGraph(),'integration.git.repository.status','Status')} readOnly={false} invalid={()=>{}} update={()=>{}}/>));
  expect(host.querySelector(`[aria-label="${t('integration.connection')}"]`)).toBeNull();
  expect(host.querySelector(`[aria-label="${t('integration.gitAuthentication')}"]`)).toBeNull();
});

it("keeps false as a boolean and offers fixed merge methods as choices", async () => {
  let current:Record<string,unknown>={};
  function Harness() {
    const [node,setNode]=useState(()=>newNode(emptyGraph(),'integration.codeup.merge_request.merge','Merge'));
    current=node.params;
    return <WorkflowAssetContext.Provider value={{workspace:'11',workflow:''}}><WorkflowNodeParameters node={node} readOnly={false} invalid={()=>{}} update={params=>setNode(value=>({...value,params}))}/></WorkflowAssetContext.Provider>;
  }
  await act(async()=>root.render(<Harness/>));
  const method=host.querySelector<HTMLSelectElement>('select[aria-label="合并方式"]');
  expect([...method!.options].map(option=>option.value)).toEqual(['','ff-only','no-fast-forward','squash','rebase']);
  await input('合并后删除源分支','false');
  expect(current.remove_source_branch).toBe(false);
  await input('合并后删除源分支','true');
  expect(current.remove_source_branch).toBe(true);
});

it("creates a connection in place, preserves failed input, then selects it without saving the outer editor",async()=>{
  let created=false,attempts=0;
  calls.mockImplementation(async(path,options)=>{
    if(String(path).endsWith('/code-connections')&&options?.method==='POST'){
      attempts++;
      if(attempts===1)throw new Error('Connection failed');
      created=true;return {id:'44',name:'Build GitLab',provider:'gitlab',state:'connected'};
    }
    if(String(path).includes('/code-connections'))return String(path).includes('/repositories?')?{items:[],has_more:false}:{items:created?[{id:'44',name:'Build GitLab',provider:'gitlab',state:'connected'}]:[],total:created?1:0,page_size:200};
    if(String(path).includes('/repository-providers'))return {items:[]};
    return {items:[{id:'8',name:'Git token',kind:'token'}],total:1,page_size:200};
  });
  let current:Record<string,unknown>={};const outerSubmit=vi.fn();
  function Harness(){const [node,setNode]=useState(()=>newNode(emptyGraph(),'integration.gitlab.project.get','Project'));current=node.params;return <form onSubmit={outerSubmit}><WorkflowAssetContext.Provider value={{workspace:'11',workflow:''}}><WorkflowNodeParameters node={node} readOnly={false} invalid={()=>{}} update={params=>setNode(value=>({...value,params}))}/></WorkflowAssetContext.Provider></form>;}
  await act(async()=>root.render(<Harness/>));
  await act(async()=>button(hostRepositoryText('repository.020')).click());
  expect(document.querySelector<HTMLSelectElement>(`[role="dialog"] [aria-label="${hostRepositoryText('repository.037')}"]`)?.disabled).toBe(true);
  await input(commonText('resource-domain:repository.connectionName'),'Build GitLab');
  await input(hostRepositoryText('repository.038'),'8');
  const submit=()=>document.querySelector('[role="dialog"] form')!.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}));
  await act(async()=>{submit();});
  expect(document.querySelector('[role="dialog"] [role="alert"]')?.textContent).toBe('Connection failed');
  expect(document.querySelector<HTMLInputElement>(`[role="dialog"] input[aria-label="${commonText('resource-domain:repository.connectionName')}"]`)?.value).toBe('Build GitLab');
  await act(async()=>{submit();});
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(current.connection_id).toBe('44');expect(current).not.toHaveProperty('credential_id');
  expect(outerSubmit).not.toHaveBeenCalled();
});
async function input(label: string, value: string) {
  const field = [
    ...document.querySelectorAll<
      HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
    >("input,select,textarea"),
  ].find((field) => field.getAttribute("aria-label") === label)!;
  expect(field, label).toBeDefined();
  await act(async () => {
    const prototype =
      field instanceof HTMLSelectElement
        ? HTMLSelectElement.prototype
        : field instanceof HTMLTextAreaElement
          ? HTMLTextAreaElement.prototype
          : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(
      field,
      value,
    );
    field.dispatchEvent(
      new Event(field instanceof HTMLSelectElement ? "change" : "input", {
        bubbles: true,
      }),
    );
  });
}

it("discovers applications and selects a specific declared operation while respecting compatible ports", async () => {
  const select = vi.fn();
  await act(async () =>
    root.render(
      <WorkflowNodePicker
        inserting={false}
        accepts={(type) => type !== "integration.github.workflow.dispatch"}
        select={select}
        close={() => {}}
      />,
    ),
  );
  await input(t("search"), "git");
  const spec = integrationManifests.find(
    (node) => node.type === "integration.github.issue.create",
  )!;
  expect(button(integrationTitle(spec))).toBeDefined();
  expect(button(integrationTitle(integrationManifests.find(node => node.ui.application === "gitlab")!))).toBeDefined();
  expect(button(integrationTitle(integrationManifests.find(node => node.ui.application === "slack")!))).toBeUndefined();
  await act(async () => button(integrationTitle(spec)).click());
  expect(select).toHaveBeenCalledWith(spec.type);
  expect(
    button(
      integrationTitle(
        integrationManifests.find(
          (node) => node.type === "integration.github.workflow.dispatch",
        )!,
      ),
    ),
  ).toBeUndefined();
});

it("creates application-bound credentials, selects remote choices and saves plain parameters without secrets", async () => {
  const spec = integrationManifests.find(
    (node) => node.type === "integration.slack.message.create",
  )!;
  const node = newNode(emptyGraph(), spec.type, "Notify");
  let current = node.params;
  function Harness() {
    const [params, update] = useState(current);
    current = params;
    return (
      <WorkflowAssetContext.Provider value={{ workspace: "11", workflow: "" }}>
        <WorkflowNodeParameters
          node={{ ...node, params }}
          update={update}
          invalid={() => {}}
          readOnly={false}
        />
      </WorkflowAssetContext.Provider>
    );
  }
  await act(async () => root.render(<Harness />));
  expect(document.querySelector('option[value="8"]')).toBeNull();
  await act(async () => button(t("integration.createCredential")).click());
  await input(t("integration.name"), "Notification connection");
  await input(t("integration.token"), "LOCAL_TEST_TOKEN");
  calls.mockResolvedValueOnce({ id: "9" });
  await act(async () =>
    document
      .querySelector<HTMLFormElement>('[role="dialog"] form')!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
  );
  expect(calls).toHaveBeenCalledWith(
    "/workspaces/11/credentials",
    expect.objectContaining({
      body: expect.stringContaining('"username":"slack"'),
    }),
  );
  expect(current.credential_id).toBe("9");
  expect(host.querySelector('option[value="9"]')?.textContent).toBe(
    "Notification connection",
  );
  expect(document.querySelector('input[type="password"]')).toBeNull();
  calls.mockResolvedValueOnce({ channels: [{ id: "C123", name: "general" }] });
  await act(async () =>
    button(
      `${t("integration.loadOptions")} · ${t("integration.channel")}`,
    ).click(),
  );
  await input(`${t("integration.channel")} · ${t("selectValue")}`, "C123");
  await input(t("integration.message"), "={{ $json.message }}");
  expect(current).toEqual({
    credential_id: "9",
    channel: "C123",
    text: "={{ $json.message }}",
  });
  expect(JSON.stringify(current)).not.toContain("LOCAL_TEST_TOKEN");
  expect(document.querySelectorAll("details[open]")).toHaveLength(0);
});

it("makes all parameter mutations unavailable in read-only mode", async () => {
  const node = newNode(
    emptyGraph(),
    "integration.github.issue.create",
    "Issue",
  );
  await act(async () =>
    root.render(
      <WorkflowNodeParameters
        node={node}
        readOnly
        update={() => {
          throw new Error("read-only mutation");
        }}
        invalid={() => {}}
      />,
    ),
  );
  expect(button(t("integration.createCredential"))).toBeUndefined();
  expect(
    [
      ...host.querySelectorAll<
        HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
      >("input,select,textarea"),
    ].every(
      (field) => field.disabled || ("readOnly" in field && field.readOnly),
    ),
  ).toBe(true);
});

it("shows credential creation errors inside the dialog and retains input for retry", async () => {
  const node = newNode(
    emptyGraph(),
    "integration.slack.message.create",
    "Notify",
  );
  await act(async () =>
    root.render(
      <WorkflowAssetContext.Provider value={{ workspace: "11", workflow: "" }}>
        <WorkflowNodeParameters
          node={node}
          update={() => {}}
          invalid={() => {}}
          readOnly={false}
        />
      </WorkflowAssetContext.Provider>,
    ),
  );
  await act(async () => button(t("integration.createCredential")).click());
  await input(t("integration.name"), "Retry connection");
  await input(t("integration.token"), "LOCAL_RETRY_TOKEN");
  calls.mockRejectedValueOnce(new Error("Request failed"));
  const submit = () =>
    document
      .querySelector<HTMLFormElement>('[role="dialog"] form')!
      .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await act(async () => {
    submit();
  });
  expect(
    document.querySelector('[role="dialog"] [role="alert"]')?.textContent,
  ).toBe("Request failed");
  expect(
    document.querySelector<HTMLInputElement>(
      '[role="dialog"] input[type="password"]',
    )?.value,
  ).toBe("LOCAL_RETRY_TOKEN");
  calls.mockResolvedValueOnce({
    id: "9",
    name: "Retry connection",
    kind: "token",
    username: "slack",
  });
  await act(async () => {
    submit();
  });
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(document.querySelector('input[type="password"]')).toBeNull();
});

it("finds the existing AI chat node by its Anthropic protocol", async () => {
  const select = vi.fn();
  await act(async () =>
    root.render(
      <WorkflowNodePicker
        inserting={false}
        accepts={() => true}
        select={select}
        close={() => {}}
      />,
    ),
  );
  await input(t("search"), "anthropic");
  expect(button(t("aiChat"))).toBeDefined();
  await act(async () => button(t("aiChat")).click());
  expect(select).toHaveBeenCalledWith("ai.llm.chat");
});

it("discards remote options returned after the selected credential changes", async () => {
  const node = newNode(
    emptyGraph(),
    "integration.slack.message.create",
    "Notify",
  );
  let finish!: (value: unknown) => void;
  function Harness() {
    const [params, update] = useState<Record<string, unknown>>({
      ...node.params,
      credential_id: "7",
    });
    return (
      <WorkflowAssetContext.Provider value={{ workspace: "11", workflow: "" }}>
        <WorkflowNodeParameters
          node={{ ...node, params }}
          update={update}
          invalid={() => {}}
          readOnly={false}
        />
      </WorkflowAssetContext.Provider>
    );
  }
  await act(async () => root.render(<Harness />));
  calls.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await act(async () =>
    button(
      `${t("integration.loadOptions")} · ${t("integration.channel")}`,
    ).click(),
  );
  const request = calls.mock.calls.at(-1)![1]!;
  await input(t("integration.credential"), "0");
  expect(request.signal?.aborted).toBe(true);
  await act(async () => finish({ channels: [{ id: "OLD", name: "Stale" }] }));
  expect(host.querySelector('option[value="OLD"]')).toBeNull();
});

it("all declared fields and operation labels are translated in all six locales", async () => {
  for (const locale of ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"]) {
    await i18n.changeLanguage(locale);
    for (const spec of integrationManifests) {
      for (const key of [
        spec.ui.resource_key,
        spec.ui.operation_key,
        spec.ui.credential_help_key,
        ...Object.values(spec.params_schema.properties).map(
          (field) => field.title_key,
        ),
      ])
        expect(integrationText(key)).not.toBe(key);
    }
  }
});

it("runtime packages join the existing picker and graph without retaining removed packages", async () => {
  const bundled = [...integrationManifests];
  const original = bundled[0]!;
  const extension = {
    ...original,
    type: "integration.ext.sample.v1.read",
    ui: {...original.ui, application:"ext.sample.v1", application_name:"Sample connector v1", resource_key:"literal:Repository", operation_key:"literal:Read", credential_help_key:"literal:Package connection", aliases:["sample"]},
  };
  try {
    installIntegrationCatalog([...bundled, extension]);
    expect(newNode(emptyGraph(), extension.type, "Sample").type).toBe(extension.type);
    expect(integrationTitle(extension)).toBe("Sample connector v1 · Repository · Read");
    await act(async () => root.render(<WorkflowNodePicker inserting={false} accepts={() => true} select={vi.fn()} close={vi.fn()} />));
    expect(button("Sample connector v1")).toBeDefined();
    await act(async () => button("Sample connector v1").click());
    await act(async () => button("Repository").click());
    expect(button("Sample connector v1 · Repository · Read")).toBeDefined();
    installIntegrationCatalog(bundled);
    expect(() => newNode(emptyGraph(), extension.type, "Removed")).toThrow("Unknown node type");
  } finally { installIntegrationCatalog(bundled); }
});


it("offers Codeup clone in its repository operations and derives its URL from the selected repository", async()=>{
  const select=vi.fn();
  await act(async()=>root.render(<WorkflowNodePicker inserting={false} accepts={()=>true} select={select} close={()=>{}}/>));
  await act(async()=>button(t('category.integrations')).click());
  await act(async()=>button('Codeup').click());
  await act(async()=>button(t('integration.resource.repository')).click());
  const clone=integrationManifests.find(node=>node.type==='integration.codeup.repository.clone')!;
  await act(async()=>button(integrationTitle(clone)).click());
  expect(select).toHaveBeenCalledWith(clone.type);
  calls.mockImplementation(async path => String(path).includes('/repositories?') ? {items:[{name:'app',full_name:'team/app',url:'https://codeup.example/team/app.git'}],has_more:false} : {items:[{id:'7',name:'Codeup shared',provider:'codeup',state:'connected'},{id:'8',name:'Other GitHub',provider:'github',state:'connected'}],total:2,page_size:200});
  let current:Record<string,unknown>={};
  function Harness(){const [node,setNode]=useState(()=>newNode(emptyGraph(),clone.type,'Clone')); current=node.params; return <WorkflowAssetContext.Provider value={{workspace:'11',workflow:''}}><WorkflowNodeParameters node={node} readOnly={false} invalid={()=>{}} update={params=>setNode(n=>({...n,params}))}/></WorkflowAssetContext.Provider>;}
  await act(async()=>root.render(<Harness/>));
  expect(host.querySelector(`[aria-label="${t('integration.connection')}"]`)?.textContent).not.toContain('Other GitHub');
  expect(host.querySelector(`input[aria-label="${t('integration.gitUrl')}"]`)).toBeNull();
  await input(t('integration.connection'),'7');
  await input(t('integration.chooseRepository'),'https://codeup.example/team/app.git');
  expect(current.connection_id).toBe('7'); expect(current.url).toBe('https://codeup.example/team/app.git');
  expect(host.textContent).toContain('https://codeup.example/team/app.git');
  expect(host.querySelector<HTMLSelectElement>(`[aria-label="${t('integration.chooseRepository')}"]`)?.value).toBe(current.url);
  expect(host.querySelector(`[aria-label="${t('integration.gitAuthentication')}"]`)).toBeNull();
});

it("uses configured branch names consistently in output labels and continuation choices",async()=>{
 const branch=newNode(emptyGraph(),'core.switch','Route');branch.params.cases=[{key:'release'},{key:'test'}];
 expect(outputPortLabel(branch,{port:'case',index:1})).toBe('test');
 expect(outputPortLabel(branch,{port:'fallback'})).toBe('未匹配');
 const condition=newNode(emptyGraph(),'core.if','If');
 expect(outputPortLabel(condition,{port:'true'})).toBe('满足条件');
 expect(outputPortLabel(condition,{port:'false'})).toBe('不满足条件');
 const change=vi.fn();
 await act(async()=>root.render(<WorkflowNodePicker inserting={false} accepts={()=>true} select={()=>{}} close={()=>{}} connection={{ports:[{port:'case',index:0,type:'main'},{port:'case',index:1,type:'main'}],value:'case:1',change,label:port=>outputPortLabel(branch,port)}}/>));
 const selector=document.querySelector<HTMLSelectElement>(`select[aria-label="${t('continueFrom')}"]`)!;
 expect(selector.selectedOptions[0]!.textContent).toBe('test');
 await input(t('continueFrom'),'case:0');expect(change).toHaveBeenCalledWith('case:0');
});
