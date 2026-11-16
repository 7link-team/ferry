import { describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { Conversation, conversationTimeline } from "./Conversation";
import * as apiModule from "../../shared/api/api";
import { conversationText } from "./resource-config";

type Message = Parameters<typeof conversationTimeline>[0][number];
type Turn = Parameters<typeof conversationTimeline>[1][number];

it("keeps reply context and draft after failure, then clears them only after a successful send", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const messages = [message("11", "user"), message("12", "assistant")];
  const row = {
    id: "2", title: "Reply test", kind: "group", state: "active", viewer_id: "2",
    can_manage: true, members: [], agents: [], messages, turns: [], version: 1,
    config: { name: "Agent", runtime: "codex", model: "", cwd: "/tmp/test" },
    binding: { execution_user: "user", canonical_directory: "/tmp/test" },
  };
  let fail = true;
  const sent: Record<string, unknown>[] = [];
  const mocked = vi.spyOn(apiModule, "api").mockImplementation(async (path, options) => {
    if (String(path).includes("/events?")) return { items: [], next: 0, has_more: false } as never;
    if (String(path).endsWith("/messages") && options?.method === "POST") {
      sent.push(JSON.parse(String(options.body)));
      if (fail) throw Error("Send failed");
      return { id: "13" } as never;
    }
    return structuredClone(row) as never;
  });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<Conversation workspace="1" id="2" permissions={new Set(["conversations.read", "conversations.run"])} initialDraft={{ prompt: "Reply draft", attachments: [] }} />));
    const reply = host.querySelector(`button[aria-label="${conversationText("reply")}"]`) as HTMLButtonElement;
    await act(async () => reply.click());
    expect(host.querySelectorAll(`button[aria-label="${conversationText("reply")}"]`)).toHaveLength(2);
    expect(host.querySelector("[data-conversation-composer] [data-conversation-reply]")?.textContent).toContain("Agent");
    const form = host.querySelector("[data-conversation-composer]")!.closest("form")!;
    await act(async () => form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(sent[0]?.reply_to).toBe("11");
    expect(host.querySelector("[data-conversation-reply]")).not.toBeNull();
    expect(host.querySelector<HTMLTextAreaElement>("#conversation-message")?.value).toBe("Reply draft");
    fail = false;
    await act(async () => form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(sent[1]?.reply_to).toBe("11");
    expect(sent[1]?.client_id).toBe(sent[0]?.client_id);
    expect(host.querySelector("[data-conversation-reply]")).toBeNull();
    expect(host.querySelector<HTMLTextAreaElement>("#conversation-message")?.value).toBe("");
  } finally {
    await act(async () => root.unmount()); host.remove(); mocked.mockRestore(); vi.unstubAllGlobals();
  }
});
function message(
  id: string,
  role: Message["role"],
  parent: string | null = null,
  turn: string | null = null,
): Message {
  return {
    id,
    role,
    reply_to: parent,
    turn_id: turn,
    agent_id: "1",
    sender_name: "Agent",
    owner_id: "2",
    prompt: "text",
    attachments: [],
    targets: [],
    created_at: 1,
  };
}
function turn(id: string): Turn {
  return {
    id,
    owner_id: "2",
    state: "running",
    prompt: "text",
    attachments: [],
    error: "",
    usage: null,
    decisions: {},
  };
}
describe("conversation timeline", () => {
  it("keeps interleaved human posts and late Agent replies in arrival order, preserving large IDs", () => {
    const first = message("112897284559077376", "user");
    const later = message("112897284559077377", "user");
    const reply = message(
      "112897284559077378",
      "assistant",
      first.id,
      "112897284559077379",
    );
    expect(
      conversationTimeline([reply, later, first], []).map(
        (row) => row.message?.id,
      ),
    ).toEqual([first.id, later.id, reply.id]);
  });
  it("shows one pending reply and replaces it with the persisted message without repeating a user post", () => {
    const pending = turn("102");
    const post = message("101", "user", null, pending.id);
    const reply = message("103", "assistant", post.id, pending.id);
    expect(
      conversationTimeline([post], [pending]).map((row) => row.id),
    ).toEqual([post.id, pending.id]);
    const entries = conversationTimeline([post, reply], [pending]);
    expect(entries.map((row) => row.id)).toEqual([post.id, reply.id]);
    expect(entries[1]?.turn?.id).toBe(pending.id);
  });
  it("renders persisted conversation text after all execution records have been removed", () => {
    const post = message("101", "user"),
      reply = message("103", "assistant", post.id, "102");
    expect(
      conversationTimeline([post, reply], []).map((row) => row.message?.prompt),
    ).toEqual([post.prompt, reply.prompt]);
  });
});

it("folds settled tool calls into one audit section and keeps pending approvals actionable", async () => {
  Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
  vi.stubGlobal("ResizeObserver",class { observe() {} disconnect() {} });
  let state="completed";
  const current={...turn("10"),usage:{input_tokens:12,output_tokens:4},decisions:{settled:{tool:"ferry_request",input:{method:"POST",path:"/workflows"},decision:true,dispatching:false,sent:true},pending:{tool:"ferry_request",input:{method:"POST",path:"/workflows/1/executions"},decision:null,dispatching:false,sent:false}}};
  const mocked=vi.spyOn(apiModule,"api").mockImplementation(async path => {
    if(String(path).includes('/events?'))return {items:[],next:0,has_more:false} as never;
    return {id:"2",title:"Audit",kind:"private",state:"active",viewer_id:"2",owner_id:"2",can_manage:true,members:[],agents:[],messages:[message("9","user"),message("11","assistant","9","10")],turns:[{...current,state}],version:1,revision_id:"1",members_version:"1",config:{name:"Agent",runtime:"codex",model:"",cwd:"/tmp"},binding:{execution_user:"user",canonical_directory:"/tmp"}} as never;
  });
  try {
    for(const next of ['completed','running']) {
      state=next;const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
      try {
        await act(()=>root.render(<Conversation workspace="1" id="2" permissions={new Set(['conversations.read','conversations.run'])} />));
        const audit=host.querySelector('details[data-execution-details]') as HTMLDetailsElement;
        expect(audit).not.toBeNull();expect(audit.open).toBe(false);
        expect(audit.textContent).toContain('ferry_request');
        const approvals=[...host.querySelectorAll('button')].filter(button=>button.textContent===conversationText('allow'));
        expect(approvals).toHaveLength(next==='running'?1:0);
        expect([...host.querySelectorAll('pre')].filter(pre=>!pre.closest('[data-execution-details]'))).toHaveLength(next==='running'?1:0);
      }finally{await act(()=>root.unmount());host.remove();}
    }
  }finally{mocked.mockRestore();vi.unstubAllGlobals();}
});

it("allows typing in an interrupted conversation and explicitly retries without replacing old attempts", async () => {
  Object.assign(globalThis, {IS_REACT_ACT_ENVIRONMENT:true});
  const original = {...turn("10"),state:"failed",prompt:"original request",error:"Native execution failed"};
  let row = {
    id:"2",title:"Interrupted chat",kind:"private",state:"interrupted",viewer_id:"2",owner_id:"2",
    can_manage:true,members:[],agents:[],messages:[],turns:[original],version:2,revision_id:"1",members_version:"1",
    config:{name:"Agent",runtime:"codex",model:"",cwd:"/tmp/test"},binding:{execution_user:"user",canonical_directory:"/tmp/test"},
  };
  const requests: {method:string;body:Record<string,unknown>}[] = [];
  const mocked = vi.spyOn(apiModule,"api").mockImplementation(async (path, options) => {
    if (String(path).includes("/events?")) return {items:[],next:0,has_more:false} as never;
    if (options?.method) {
      const body=JSON.parse(String(options.body)); requests.push({method:options.method,body});
      if(options.method==="PATCH") row={...row,state:"active",version:3};
      return {id:"11"} as never;
    }
    return structuredClone(row) as never;
  });
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  try {
    await act(async()=>root.render(<Conversation workspace="1" id="2" permissions={new Set(["conversations.read","conversations.run","conversations.update"])} initialDraft={{prompt:"unsent draft",attachments:[]}} />));
    expect(host.querySelector("textarea")?.disabled).toBe(false);
    const retry=[...host.querySelectorAll("button")].find(b=>b.textContent?.includes(conversationText("retryLastTurn")));
    expect(retry).toBeDefined();
    const failure = retry!.closest("[data-turn-failure]");
    expect(failure?.getAttribute("data-turn-failure")).toBe(original.id);
    expect(failure?.textContent).toContain(original.error);
    expect(host.textContent).not.toContain(conversationText("interruptedHelp"));
    await act(async()=>retry!.click());
    expect(requests.map(r=>r.method)).toEqual(["PATCH","POST"]);
    expect(requests[0]?.body).toMatchObject({state:"active",version:2});
    expect(requests[1]?.body).toMatchObject({prompt:"original request"});
    expect(requests[1]?.body.client_id).toBeTruthy();
    expect(original.state).toBe("failed");
    expect(host.querySelector("textarea")?.value).toBe("unsent draft");
  } finally {
    await act(async()=>root.unmount());host.remove();mocked.mockRestore();
  }
});

it("shows the bound host, platform notices, and offers another host when a chosen directory's host is down", async () => {
  Object.assign(globalThis, {IS_REACT_ACT_ENVIRONMENT:true});
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const failed = {...turn("10"),state:"failed",prompt:"build it",error:"Execution host alpha cannot run this now: offline or unreachable.",error_key:"conversations.hostUnavailable"};
  const notice = {...message("12","notice"),prompt:"Switched execution from host alpha to beta."};
  let row: Record<string, unknown> = {
    id:"2",title:"Pinned dir",kind:"private",state:"active",viewer_id:"2",owner_id:"2",
    can_manage:true,members:[],agents:[],messages:[notice],turns:[failed],version:2,revision_id:"1",members_version:"1",
    config:{name:"Agent",runtime:"codex",model:"",cwd:"/srv/repo"},binding:{execution_user:"user",canonical_directory:"/srv/repo"},
    host:{id:"31",name:"alpha"},
  };
  const posts: {path:string;body:unknown}[] = [];
  const mocked = vi.spyOn(apiModule,"api").mockImplementation(async (path, options) => {
    if (String(path).includes("/events?")) return {items:[],next:0,has_more:false} as never;
    if (options?.method === "POST") {
      posts.push({path:String(path),body:JSON.parse(String(options.body))});
      row = {...row,binding:null,host:null};
      return {ok:true} as never;
    }
    return structuredClone(row) as never;
  });
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  try {
    await act(async()=>root.render(<Conversation workspace="1" id="2" permissions={new Set(["conversations.read","conversations.run"])} />));
    expect(host.querySelector("header")?.textContent).toContain(`${conversationText("executionHost")} alpha`);
    const shown = host.querySelector("[data-conversation-notice]");
    expect(shown?.textContent).toBe("Switched execution from host alpha to beta.");
    const another = [...host.querySelectorAll("button")].find(b=>b.textContent===conversationText("useAnotherHost"));
    expect(another).toBeDefined();
    await act(async()=>another!.click());
    expect(posts).toEqual([{path:"/workspaces/1/conversations/2/host-releases",body:{}}]);
    // Released: no host until the next message chooses one.
    expect(host.querySelector("header")?.textContent).toContain(`${conversationText("executionHost")} ${conversationText("hostPending")}`);
  } finally {
    await act(async()=>root.unmount());host.remove();mocked.mockRestore();vi.unstubAllGlobals();
  }
});

it("says when the Agent has no model instead of claiming a default model", async () => {
  Object.assign(globalThis, {IS_REACT_ACT_ENVIRONMENT:true});
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  let row: Record<string, unknown> = {
    id:"2",title:"No model",kind:"private",state:"active",viewer_id:"2",owner_id:"2",agent_has_model:false,
    can_manage:true,members:[],agents:[],messages:[],turns:[],version:2,revision_id:"1",members_version:"1",
    config:{name:"Agent",runtime:"codex",model:"",cwd:""},binding:null,host:{id:"31",name:"alpha"},
  };
  const mocked = vi.spyOn(apiModule,"api").mockImplementation(async (path) => {
    if (String(path).includes("/events?")) return {items:[],next:0,has_more:false} as never;
    return structuredClone(row) as never;
  });
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  try {
    await act(async()=>root.render(<Conversation workspace="1" id="2" permissions={new Set(["conversations.read"])} />));
    const header = host.querySelector("header")?.textContent ?? "";
    expect(header).toContain(conversationText("noModel"));
    expect(header).not.toContain(conversationText("nativeModel"));
    // With a configured model and no explicit choice, the model's default is used.
    row = {...row,agent_has_model:true,version:3};
    await act(async()=>root.render(<Conversation key="again" workspace="1" id="2" permissions={new Set(["conversations.read"])} />));
    expect(host.querySelector("header")?.textContent).toContain(conversationText("nativeModel"));
    expect(conversationText("nativeModel")).not.toContain("原生");
  } finally {
    await act(async()=>root.unmount());host.remove();mocked.mockRestore();vi.unstubAllGlobals();
  }
});

it("offers another host only for that failure", async () => {
  Object.assign(globalThis, {IS_REACT_ACT_ENVIRONMENT:true});
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const failed = {...turn("10"),state:"failed",prompt:"x",error:"No execution host can run this now",error_key:"models.noHost"};
  const row = {
    id:"2",title:"Auto",kind:"private",state:"active",viewer_id:"2",owner_id:"2",
    can_manage:true,members:[],agents:[],messages:[],turns:[failed],version:2,revision_id:"1",members_version:"1",
    config:{name:"Agent",runtime:"codex",model:"",cwd:""},binding:null,host:null,
  };
  const mocked = vi.spyOn(apiModule,"api").mockImplementation(async (path) => {
    if (String(path).includes("/events?")) return {items:[],next:0,has_more:false} as never;
    return structuredClone(row) as never;
  });
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  try {
    await act(async()=>root.render(<Conversation workspace="1" id="2" permissions={new Set(["conversations.read","conversations.run"])} />));
    expect(host.textContent).toContain("No execution host can run this now");
    expect([...host.querySelectorAll("button")].some(b=>b.textContent===conversationText("useAnotherHost"))).toBe(false);
  } finally {
    await act(async()=>root.unmount());host.remove();mocked.mockRestore();vi.unstubAllGlobals();
  }
});
