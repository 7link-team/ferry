import { describe, expect, it } from "vitest";
import { conversationTimeline } from "./Conversation";

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
