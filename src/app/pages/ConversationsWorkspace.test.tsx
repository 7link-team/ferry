import { afterEach, expect, it, vi } from "vitest";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { conversationText } from "../../features/conversations/resource-config";
import { listResource } from "../../shared/api/resource";
import type { ConversationsSession } from "./ConversationsWorkspace";
import { ApiError } from "../../shared/api/api";
import { observeSession, clearSession } from "../../shared/api/session";

const route = vi.hoisted(() => ({ mode: "view", id: "1" as string | undefined }));
afterEach(() => { route.mode = "view"; route.id = "1"; clearSession(); });

vi.mock("../router", () => ({
  href: () => "#",
  navigate: () => undefined,
  useRoute: () => route,
}));
const activeConversation = vi.hoisted(() => ({ id: "", onPendingChange: undefined as undefined | ((id: string, count: number) => void) }));
vi.mock("../../features/conversations/Conversation", () => ({ Conversation: (props: typeof activeConversation) => {
  activeConversation.id = props.id;
  activeConversation.onPendingChange = props.onPendingChange;
  return <div data-conversation-content>Existing conversation</div>;
} }));
vi.mock("../../shared/api/resource", () => ({
  listResource: vi.fn(async () => ({
    items: [
      { id: "1", title: "Private chat", agent_name: "Agent", state: "active", kind: "private", pending: 2 },
      { id: "2", title: "Team room", agent_name: "", state: "active", kind: "group", pending: 1, unread: 3 },
      { id: "3", title: "Quiet", agent_name: "Agent", state: "active", kind: "private", pending: 0 },
    ],
    total: 3,
    page: 1,
    page_size: 50,
  })),
}));

it("shows loading rather than a new-chat empty state before the first list response", async () => {
  route.mode = "list"; route.id = undefined;
  vi.mocked(listResource).mockImplementationOnce(() => new Promise(() => {}));
  const { ConversationsWorkspace } = await import("./ConversationsWorkspace");
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<ConversationsWorkspace workspace="1" permissions={new Set(["conversations.read"])} form={undefined} />));
    expect(host.querySelector('main [data-slot="loading-skeleton"]')).not.toBeNull();
    expect(host.textContent).not.toContain(conversationText("startConversation"));
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it.each(["2", "older-unlisted-room"])("restores last conversation %s immediately while revalidating and retains it offline", async selected => {
  route.mode = "list"; route.id = undefined;
  const session: ConversationsSession = { drafts: new Map(), selected, list:{rows:[{id:"2",title:"Saved room",agent_name:"Agent",state:"active"}],total:1,query:"",pages:1} };
  vi.mocked(listResource).mockRejectedValueOnce(new Error("Offline"));
  const { ConversationsWorkspace } = await import("./ConversationsWorkspace");
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<ConversationsWorkspace workspace="1" permissions={new Set(["conversations.read"])} form={undefined} session={session} />));
    expect(host.querySelector('main [data-slot="loading-skeleton"]')).toBeNull();
    expect(host.querySelector('[data-conversation-content]')).not.toBeNull();
    expect(activeConversation.id).toBe(selected);
    expect(host.textContent).toContain("Saved room"); expect(host.textContent).toContain("Offline");
    expect(host.textContent).not.toContain(conversationText("startConversation"));
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("removes cached room content and drafts when list access is revoked", async () => {
  route.mode = "list"; route.id = undefined;
  const session: ConversationsSession = { drafts:new Map([["2",{prompt:"private draft",attachments:[],targets:[],mentions:[],reply_to:""}]]), selected:"2", list:{rows:[{id:"2",title:"Private room",agent_name:"Agent",state:"active"}],total:1,query:"",pages:1} };
  vi.mocked(listResource).mockRejectedValueOnce(new ApiError("Access denied",403));
  const { ConversationsWorkspace } = await import("./ConversationsWorkspace");
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  try {
    await act(async()=>root.render(<ConversationsWorkspace workspace="1" permissions={new Set(["conversations.read"])} form={undefined} session={session} />));
    expect(host.querySelector('[data-conversation-content]')).toBeNull();
    expect(host.textContent).not.toContain("Private room");
    expect(session.list).toBeUndefined();expect(session.snapshot).toBeUndefined();expect(session.drafts.size).toBe(0);
  } finally { await act(async()=>root.unmount());host.remove(); }
});

it("preserves the selected room and draft through renewable 401 and recovers after renewal", async () => {
  Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
  const now = Date.now();
  observeSession({server_time_ms:now,access_expires_at_ms:now-1,refresh_expires_at_ms:now+60000,auto_renew:true,refreshable:true});
  const session: ConversationsSession = {drafts:new Map([['1',{prompt:'Keep draft',attachments:[]}]]),selected:'1',list:{rows:[{id:'1',title:'Saved room',agent_name:'Agent',state:'active'}],total:1,query:'',pages:1}};
  vi.mocked(listResource).mockRejectedValueOnce(new ApiError('Sign in',401));
  const { ConversationsWorkspace } = await import('./ConversationsWorkspace');
  const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
  try {
    await act(async()=>root.render(<ConversationsWorkspace workspace="1" permissions={new Set(['conversations.read'])} form={undefined} session={session}/>));
    expect(host.querySelector('[data-conversation-content]')).not.toBeNull();
    expect(session.drafts.get('1')?.prompt).toBe('Keep draft');
    await act(async()=>window.dispatchEvent(new Event('session:renewed')));
    expect(host.textContent).toContain('Private chat');
    expect(host.textContent).not.toContain('Sign in');
  } finally {await act(async()=>root.unmount());host.remove();}
});

it("badges private and group conversations that have approvals waiting for the viewer", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const { ConversationsWorkspace } = await import("./ConversationsWorkspace");
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<ConversationsWorkspace workspace="1" permissions={new Set(["conversations.read"])} form={undefined} />));
    const badges = [...host.querySelectorAll("[data-pending-badge]")];
    expect(badges.map((badge) => badge.textContent)).toEqual(["2", "1"]);
    expect(badges[0]?.getAttribute("aria-label")).toBe(`${conversationText("pendingFilter")} 2`);
    expect(badges[1]?.closest("a")?.textContent).toContain("Team room");
    // The active conversation takes precedence over a stale list snapshot.
    expect(activeConversation.onPendingChange).toBeDefined();
    await act(async () => activeConversation.onPendingChange!("1", 1));
    expect([...host.querySelectorAll("[data-pending-badge]")].map(badge => badge.textContent)).toEqual(["1", "1"]);
    await act(async () => activeConversation.onPendingChange!("1", 0));
    expect([...host.querySelectorAll("[data-pending-badge]")].map(badge => badge.textContent)).toEqual(["1"]);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
