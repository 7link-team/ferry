import { describe, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { Conversation, conversationTimeline } from "./Conversation";
import * as apiModule from "../../shared/api/api";
import { conversationText } from "./resource-config";

type Message = Parameters<typeof conversationTimeline>[0][number];
type Turn = Parameters<typeof conversationTimeline>[1][number];
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
