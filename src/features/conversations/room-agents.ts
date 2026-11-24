// Rooms can hold the same Agent once per developer (its sponsor). The server
// keeps `id` as the Agent definition for older clients and adds the room row;
// the conversation UI works with room rows so copies stay distinct.

export interface RoomAgentFields {
  id: string;
  name: string;
  room_agent_id?: string;
  /** The Agent definition, when `id` has been replaced by the room row. */
  agent_id?: string;
  added_by?: string;
  sponsor_name?: string;
  approval_mode?: string;
}
interface RoomTurn { id: string; agent_id?: string; room_agent_id?: string; message_id?: string }
interface RoomMessage { id: string; turn_id: string | null; agent_id: string; owner_id: string; targets: string[] }
interface RoomData<A, T, M> { kind: "private" | "group"; agents: A[]; turns: T[]; messages: M[] }

/** Label shown for a room Agent: its name and who added it. */
export function roomAgentLabel(agent: Pick<RoomAgentFields, "name" | "sponsor_name">) {
  return agent.sponsor_name ? `${agent.name} (${agent.sponsor_name})` : agent.name;
}

export function normalizeRoom<A extends RoomAgentFields, T extends RoomTurn, M extends RoomMessage, D extends RoomData<A, T, M>>(data: D): D {
  if (data.kind !== "group" || !data.agents.some(agent => agent.room_agent_id)) return data;
  const agents = data.agents.map(agent => agent.room_agent_id
    ? { ...agent, id: agent.room_agent_id, agent_id: agent.id, name: roomAgentLabel(agent) }
    : agent);
  const turns = data.turns.map(turn => turn.room_agent_id ? { ...turn, agent_id: turn.room_agent_id } : turn);
  const rowOf = (definition: string, sponsor?: string) =>
    agents.find(agent => agent.agent_id === definition && (!sponsor || agent.added_by === sponsor))?.id;
  const messages = data.messages.map(message => {
    const turn = message.turn_id ? turns.find(item => item.id === message.turn_id) : undefined;
    const agent = message.agent_id && message.agent_id !== "0"
      ? turn?.agent_id ?? rowOf(message.agent_id, message.owner_id) ?? rowOf(message.agent_id) ?? message.agent_id
      : message.agent_id;
    // The turns a message started name exactly which copies it addressed.
    const addressed = turns.filter(item => item.message_id === message.id && item.agent_id).map(item => item.agent_id!);
    const targets = addressed.length ? [...new Set(addressed)] : message.targets.flatMap(id => {
      const rows = agents.filter(item => item.agent_id === id).map(item => item.id);
      return rows.length ? rows : [id];
    });
    return { ...message, agent_id: agent, targets };
  });
  return { ...data, agents, turns, messages };
}
