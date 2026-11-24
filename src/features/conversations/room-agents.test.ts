import { expect, it } from "vitest";
import { normalizeRoom, roomAgentLabel, type RoomAgentFields } from "./room-agents";

const room = {
  kind: "group" as const,
  agents: [
    { id: "7", name: "Claude Code", room_agent_id: "101", added_by: "1", sponsor_name: "Ana" },
    { id: "7", name: "Claude Code", room_agent_id: "102", added_by: "2", sponsor_name: "Bo" },
  ] as RoomAgentFields[],
  turns: [{ id: "t1", agent_id: "7", room_agent_id: "102", message_id: "m3" }],
  messages: [
    { id: "m1", role: "user", turn_id: null, agent_id: "0", owner_id: "1", targets: ["7"] },
    { id: "m2", role: "assistant", turn_id: "t1", agent_id: "7", owner_id: "2", targets: [] },
    { id: "m3", role: "relay", turn_id: null, agent_id: "7", owner_id: "1", targets: ["7"] },
  ],
};

it("keeps two developers' copies of one Agent distinct by room row", () => {
  const data = normalizeRoom(room);
  expect(data.agents.map(agent => [agent.id, agent.agent_id, agent.name])).toEqual([
    ["101", "7", "Claude Code (Ana)"],
    ["102", "7", "Claude Code (Bo)"],
  ]);
  expect(data.turns[0]!.agent_id).toBe("102");
  // A reply belongs to its turn's row; a relay to its sponsor's copy.
  expect(data.messages[1]!.agent_id).toBe("102");
  expect(data.messages[2]!.agent_id).toBe("101");
  // Without turns, a definition target may mean either copy; with turns, exactly the addressed one.
  expect(data.messages[0]!.targets).toEqual(["101", "102"]);
  expect(data.messages[2]!.targets).toEqual(["102"]);
});

it("leaves private conversations and older servers unchanged", () => {
  const legacy = { ...room, agents: room.agents.map(({ room_agent_id: _row, ...agent }) => agent) };
  expect(normalizeRoom(legacy)).toBe(legacy);
  const priv = { ...room, kind: "private" as const };
  expect(normalizeRoom(priv)).toBe(priv);
  expect(roomAgentLabel({ name: "Codex" })).toBe("Codex");
});
