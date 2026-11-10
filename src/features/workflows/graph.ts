import { insertLayout } from "../../shared/editor/insertion-layout";
import core from "../../shared/workflow-contracts/workflow-core-nodes.json";
import data from "../../shared/workflow-contracts/workflow-data-nodes.json";
import file from "../../shared/workflow-contracts/workflow-file-nodes.json";
import ai from "../../shared/workflow-contracts/workflow-ai-nodes.json";
import integrations from "../../shared/workflow-contracts/workflow-integration-nodes.json";

export interface Port {
  port: string;
  type: string;
  index?: number;
}
export interface Manifest {
  item_mode?: "per_item" | "batch";
  search_aliases?: string[];
  type: string;
  type_version: number;
  inputs: Port[];
  outputs: Port[];
  example_params: Record<string, unknown>;
  params_schema: {
    properties?: Record<
      string,
      { type?: string; enum?: unknown[]; minimum?: number; maximum?: number }
    >;
    required?: string[];
  };
}
export interface GraphNode {
  id: string;
  name: string;
  type: string;
  type_version: number;
  params: Record<string, unknown>;
  position: [number, number];
  settings?: Record<string, unknown>;
  [key: string]: unknown;
}
export interface Endpoint {
  node: string;
  port: string;
  index: number;
}
export interface Link {
  from: Endpoint;
  to: Endpoint;
  [key: string]: unknown;
}
export interface Graph {
  engine: "ferry-graph";
  schema: 1;
  nodes: GraphNode[];
  connections: Link[];
  [key: string]: unknown;
}
export const manifests = [
  ...core.nodes,
  ...data.nodes,
  ...file.nodes,
  ...ai.nodes,
  ...integrations.nodes,
] as unknown as Manifest[];
const bundledManifestCount = manifests.length;
export function setExtensionManifests(nodes: Manifest[]) {
  manifests.splice(bundledManifestCount, manifests.length - bundledManifestCount, ...nodes);
}
/** Route saved and new connections according to their card positions. */
export function connectionDirection(
  graph: Graph,
  link: Link,
): "right" | "down" {
  const from = graph.nodes.find((node) => node.id === link.from.node);
  const to = graph.nodes.find((node) => node.id === link.to.node);
  return from &&
    to &&
    to.position[1] > from.position[1] &&
    Math.abs(to.position[0] - from.position[0]) < 224
    ? "down"
    : "right";
}
export const manifest = (type: string) =>
  manifests.find((node) => node.type === type);

export function parseGraph(source: string): Graph {
  const value = JSON.parse(source) as Graph;
  if (
    value.engine !== "ferry-graph" ||
    value.schema !== 1 ||
    !Array.isArray(value.nodes) ||
    !Array.isArray(value.connections)
  )
    throw new Error("Invalid workflow graph");
  return value;
}
export const emptyGraph = (): Graph => ({
  engine: "ferry-graph",
  schema: 1,
  nodes: [],
  connections: [],
});
export function newNode(graph: Graph, type: string, name: string): GraphNode {
  const spec = manifest(type);
  if (!spec) throw new Error("Unknown node type");
  const base = type.replaceAll(".", "_").slice(0, 56);
  let id = base,
    number = 2;
  while (graph.nodes.some((node) => node.id === id)) id = `${base}_${number++}`;
  const defaults = structuredClone(spec.example_params);
  if (spec.params_schema.properties?.auth_mode && spec.params_schema.properties?.connection_id) {
    // Examples describe the schema; a new node must not select a sample connection/repository.
    delete defaults.connection_id;
    delete defaults.url;
  }
  // The example names a sample host; a new preview must pick its own.
  if (type === "host.preview") defaults.host_id = "";
  if (type === "notification.send") defaults.channel_id = "";
  if (spec.params_schema.properties?.connection_id && spec.params_schema.properties?.credential_id) {
    delete defaults.credential_id;
    delete defaults.organization_id;
  }
  return {
    id,
    name,
    type,
    type_version: spec.type_version,
    params:
      type === "file.decode"
        ? {
            ...structuredClone(spec.example_params),
            format: "text",
            asset: "={{ $binary.file.asset }}",
          }
        : defaults,
    position: [
      graph.nodes.length
        ? Math.max(...graph.nodes.map((node) => node.position[0])) + 320
        : 0,
      160,
    ],
  };
}
export function ports(
  node: GraphNode,
  direction: "inputs" | "outputs",
): Port[] {
  const spec = manifest(node.type);
  if (!spec) return [];
  if (node.type === "core.switch" && direction === "outputs") {
    const cases = node.params.cases as { key: string }[];
    return [
      ...(cases ?? []).map((_, index) => ({
        port: "case",
        type: "main",
        index,
      })),
      ...(node.params.fallback ? [{ port: "fallback", type: "main" }] : []),
    ];
  }
  // Review mode decides the exits: none -> main; human -> approved / rejected.
  if (node.type === "host.preview" && direction === "outputs")
    return spec.outputs.filter((port) =>
      node.params.review === "human" ? port.port !== "main" : port.port === "main",
    );
  if (node.type === "core.merge" && direction === "inputs")
    return Array.from(
      { length: Number(node.params.input_count ?? 2) },
      (_, index) => ({ port: "input", type: "main", index }),
    );
  return spec[direction];
}
/** Preserve the identity of indexed branches when removing cases in the form. */
export function editGraphNode(
  graph: Graph,
  id: string,
  patch: Partial<GraphNode>,
): Graph {
  const original = graph.nodes.find((node) => node.id === id);
  if (!original) return graph;
  const updated = { ...original, ...patch };
  const oldCases =
    original.type === "core.switch"
      ? (original.params.cases as { key: string }[])
      : [];
  const newCases =
    updated.type === "core.switch"
      ? (updated.params.cases as { key: string }[])
      : [];
  const connections = graph.connections.flatMap((link) => {
    let next = link;
    if (
      link.from.node === id &&
      link.from.port === "case" &&
      oldCases.length !== newCases.length
    ) {
      const key = oldCases[link.from.index]?.key;
      const index = newCases.findIndex((branch) => branch.key === key);
      if (index < 0) return [];
      next = { ...link, from: { ...link.from, index } };
    }
    for (const [side, direction] of [
      ["from", "outputs"],
      ["to", "inputs"],
    ] as const) {
      if (
        next[side].node === id &&
        !ports(updated, direction).some(
          (port) =>
            port.port === next[side].port &&
            (port.index ?? 0) === next[side].index,
        )
      )
        return [];
    }
    return [next];
  });
  return {
    ...graph,
    nodes: graph.nodes.map((node) => (node.id === id ? updated : node)),
    connections,
  };
}

function port(
  graph: Graph,
  endpoint: Endpoint,
  direction: "inputs" | "outputs",
) {
  const node = graph.nodes.find((node) => node.id === endpoint.node);
  return (
    node &&
    ports(node, direction).find(
      (port) =>
        port.port === endpoint.port && (port.index ?? 0) === endpoint.index,
    )
  );
}
export function canConnect(
  graph: Graph,
  from: Endpoint,
  to: Endpoint,
): boolean {
  const output = port(graph, from, "outputs"),
    input = port(graph, to, "inputs");
  if (!output || !input || output.type !== input.type || from.node === to.node)
    return false;
  const same = (a: Endpoint, b: Endpoint) =>
    a.node === b.node && a.port === b.port && a.index === b.index;
  if (
    graph.connections.some((link) => same(link.from, from) && same(link.to, to))
  )
    return false;
  const visited = new Set<string>();
  function reaches(id: string): boolean {
    if (id === from.node) return true;
    if (visited.has(id)) return false;
    visited.add(id);
    return graph.connections
      .filter((link) => link.from.node === id)
      .some((link) => reaches(link.to.node));
  }
  return !reaches(to.node);
}

/** Place a connected successor nearby without moving the existing graph. */
export function addAdjacentNode(
  graph: Graph,
  from: Endpoint,
  direction: "right" | "down",
  node: GraphNode,
  sizes: Record<string, { width: number; height: number }> = {},
): Graph {
  const source = graph.nodes.find((item) => item.id === from.node);
  const output = port(graph, from, "outputs");
  const terminal = source && ports(source, "outputs").length === 0;
  const inputs = ports(node, "inputs").filter(
    (input) => input.type === output?.type,
  );
  if (
    !source ||
    (!terminal && (!output || inputs.length !== 1)) ||
    graph.nodes.some((item) => item.id === node.id)
  )
    throw new Error("Incompatible adjacent node");
  const size = (item: GraphNode) =>
    sizes[item.id] ?? {
      width: 224,
      height: Math.max(
        160,
        ports(item, "outputs").length > 1 ? ports(item, "outputs").length * 44 + 140 : 0,
        Math.max(ports(item, "inputs").length, ports(item, "outputs").length) *
          30 +
          72,
      ),
    };
  const placed = {
    ...node,
    position: [...source.position] as [number, number],
  };
  const axis = direction === "right" ? 0 : 1;
  placed.position[axis] +=
    (direction === "right" ? size(source).width : size(source).height) + 96;
  const candidate = size(placed);
  while (
    graph.nodes.some((item) => {
      const other = size(item);
      return (
        placed.position[0] < item.position[0] + other.width + 48 &&
        placed.position[0] + candidate.width + 48 > item.position[0] &&
        placed.position[1] < item.position[1] + other.height + 48 &&
        placed.position[1] + candidate.height + 48 > item.position[1]
      );
    })
  )
    placed.position[axis] +=
      (direction === "right" ? candidate.width : candidate.height) + 96;
  const input = inputs[0]!;
  const next = { ...graph, nodes: [...graph.nodes, placed] };
  if (terminal) return next;
  const to = { node: node.id, port: input.port, index: input.index ?? 0 };
  if (!canConnect(next, from, to))
    throw new Error("Invalid adjacent connection");
  return { ...next, connections: [...graph.connections, { from, to }] };
}
export function insertionPorts(
  graph: Graph,
  link: Link,
  type: string,
): { input: Port; output: Port } | undefined {
  const candidate = newNode(graph, type, type);
  const before = port(graph, link.from, "outputs"),
    after = port(graph, link.to, "inputs");
  const inputs = ports(candidate, "inputs").filter(
    (input) => input.type === before?.type,
  );
  const outputs = ports(candidate, "outputs").filter(
    (output) => output.type === after?.type,
  );
  // Multiple ports require explicit wiring; never silently select a branch.
  return inputs.length === 1 && outputs.length === 1
    ? { input: inputs[0]!, output: outputs[0]! }
    : undefined;
}
export function insertNode(
  graph: Graph,
  index: number,
  node: GraphNode,
  sizes: Record<string, { width: number; height: number }> = {},
): Graph {
  const link = graph.connections[index];
  if (!link) throw new Error("Connection no longer exists");
  const compatible = insertionPorts(graph, link, node.type);
  if (!compatible || graph.nodes.some((existing) => existing.id === node.id))
    throw new Error("Incompatible insertion");
  const size = (item: GraphNode) => sizes[item.id] ?? { width: 224, height: Math.max(160, Math.max(ports(item, "inputs").length, ports(item, "outputs").length) * 30 + 72) };
  const positions = insertLayout(graph.nodes.map(item => ({ id:item.id,x:item.position[0],y:item.position[1],...size(item) })), graph.connections.map(edge => ({from:edge.from.node,to:edge.to.node})), link.from.node, link.to.node, {id:node.id,x:0,y:0,...size(node)}, connectionDirection(graph,link));
  return {
    ...graph,
    nodes: [...graph.nodes,node].map(item => ({...item,position:[positions[item.id]!.x,positions[item.id]!.y] as [number,number]})),
    connections: graph.connections.flatMap((existing, at) =>
      at !== index
        ? [existing]
        : [
            {
              ...existing,
              to: {
                node: node.id,
                port: compatible.input.port,
                index: compatible.input.index ?? 0,
              },
            },
            {
              from: {
                node: node.id,
                port: compatible.output.port,
                index: compatible.output.index ?? 0,
              },
              to: existing.to,
            },
          ],
    ),
  };
}

/** Reflow the DAG without turning explicitly downward successors into horizontal ones. */
export function arrangeGraph(graph: Graph, sizes: Record<string, {width: number; height: number}> = {}): Graph {
  const incoming = new Map(
    graph.nodes.map((node) => [node.id, new Set<string>()]),
  );
  const outgoing = new Map(
    graph.nodes.map((node) => [node.id, new Set<string>()]),
  );
  for (const link of graph.connections) {
    incoming.get(link.to.node)?.add(link.from.node);
    outgoing.get(link.from.node)?.add(link.to.node);
  }
  const remaining = new Map(
    [...incoming].map(([id, parents]) => [id, parents.size]),
  );
  const ranks = new Map(graph.nodes.map((node) => [node.id, 0]));
  const queue = graph.nodes
    .filter((node) => !remaining.get(node.id))
    .map((node) => node.id);
  for (let index = 0; index < queue.length; index++) {
    const id = queue[index]!;
    for (const child of outgoing.get(id) ?? []) {
      ranks.set(
        child,
        Math.max(ranks.get(child) ?? 0, (ranks.get(id) ?? 0) + 1),
      );
      remaining.set(child, (remaining.get(child) ?? 0) - 1);
      if (!remaining.get(child)) queue.push(child);
    }
  }
  // Invalid cyclic drafts remain editable; arranging them must not hang or drop nodes.
  if (queue.length !== graph.nodes.length) return graph;
  const positions = new Map<string, [number, number]>();
  const height = (node: GraphNode) => sizes[node.id]?.height ?? Math.max(160, Math.max(ports(node, "inputs").length, ports(node, "outputs").length) * 30 + 72, ports(node, "outputs").length > 1 ? ports(node, "outputs").length * 44 + 140 : 0);
  if (graph.connections.some(link => connectionDirection(graph, link) === "down")) {
    const placed: GraphNode[] = [];
    for (const id of queue) {
      const node = graph.nodes.find(item => item.id === id)!;
      const links = graph.connections.filter(link => link.to.node === id);
      let x = links.length ? 0 : Math.max(0, node.position[0]);
      let y = links.length ? 80 : Math.max(80, node.position[1]);
      for (const link of links) {
        const parent = placed.find(item => item.id === link.from.node);
        if (!parent) continue;
        const down = connectionDirection(graph, link) === "down";
        x = Math.max(x, parent.position[0] + (down ? 0 : 320));
        y = Math.max(y, parent.position[1] + (down ? height(parent) + 64 : 0));
      }
      // Resolve collisions by moving down, retaining the chosen horizontal column.
      while (placed.some(other => Math.abs(x - other.position[0]) < 272 && y < other.position[1] + height(other) + 64 && y + height(node) + 64 > other.position[1])) {
        y = Math.max(...placed.filter(other => Math.abs(x - other.position[0]) < 272 && y < other.position[1] + height(other) + 64 && y + height(node) + 64 > other.position[1]).map(other => other.position[1] + height(other) + 64));
      }
      placed.push({...node, position: [x, y]});
      positions.set(id, [x, y]);
    }
    return {...graph, nodes: graph.nodes.map(node => ({...node, position: positions.get(node.id)!}))};
  }
  for (let rank = 0; rank <= Math.max(0, ...ranks.values()); rank++) {
    let bottom = 80;
    const preferred = (node: GraphNode) => {
      const parents = [...(incoming.get(node.id) ?? [])]
        .map((id) => positions.get(id)?.[1])
        .filter((y): y is number => y !== undefined);
      return parents.length
        ? parents.reduce((sum, y) => sum + y, 0) / parents.length
        : node.position[1];
    };
    for (const node of graph.nodes
      .filter((node) => ranks.get(node.id) === rank)
      .sort((a, b) => preferred(a) - preferred(b))) {
      const y = Math.max(bottom, preferred(node));
      positions.set(node.id, [rank * 320, y]);
      bottom =
        y +
        height(node) +
        64;
    }
  }
  return {
    ...graph,
    nodes: graph.nodes.map((node) => ({
      ...node,
      position: positions.get(node.id)!,
    })),
  };
}
