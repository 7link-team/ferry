import { describe, expect, it } from "vitest";
import {
  arrangeGraph,
  addAdjacentNode,
  connectionDirection,
  editGraphNode,
  canConnect,
  emptyGraph,
  insertNode,
  insertionPorts,
  newNode,
  ports,
} from "./graph";

function fixture() {
  const graph = emptyGraph();
  graph.nodes = [newNode(graph, "core.trigger.manual", "Start")];
  graph.nodes.push(newNode(graph, "data.set", "Set"));
  graph.connections = [
    {
      from: { node: graph.nodes[0]!.id, port: "main", index: 0 },
      to: { node: graph.nodes[1]!.id, port: "main", index: 0 },
    },
  ];
  return graph;
}
describe("workflow connection editing", () => {
  it("adds successors right or below without moving existing nodes or overlapping them", () => {
    const original = fixture();
    const source = original.nodes[0]!;
    const from = { node: source.id, port: "main", index: 0 };
    for (const direction of ["right", "down"] as const) {
      let graph = structuredClone(original);
      for (let count = 0; count < 3; count++) {
        const candidate = newNode(graph, "core.no_op", "Next");
        graph = addAdjacentNode(graph, from, direction, candidate);
        const placed = graph.nodes.at(-1)!;
        expect(placed.position[direction === "right" ? 0 : 1]).toBeGreaterThan(
          source.position[direction === "right" ? 0 : 1],
        );
        for (const other of graph.nodes.slice(0, -1)) {
          expect(
            Math.abs(placed.position[0] - other.position[0]) >= 272 ||
              Math.abs(placed.position[1] - other.position[1]) >= 208,
          ).toBe(true);
        }
        expect(graph.connections.at(-1)).toEqual({
          from,
          to: { node: placed.id, port: "main", index: 0 },
        });
      }
      expect(graph.nodes.slice(0, original.nodes.length)).toEqual(
        original.nodes,
      );
      expect(graph.connections[0]).toEqual(original.connections[0]);
    }
  });
  it("continues the chosen branch and rejects incompatible or ambiguous inputs", () => {
    const graph = fixture();
    const branch = newNode(graph, "core.if", "Branch");
    graph.nodes.push(branch);
    const from = { node: branch.id, port: "false", index: 0 };
    const added = addAdjacentNode(
      graph,
      from,
      "down",
      newNode(graph, "data.set", "Set"),
    );
    expect(added.connections.at(-1)!.from).toEqual(from);
    for (const type of ["core.trigger.manual", "core.merge"])
      expect(() =>
        addAdjacentNode(graph, from, "right", newNode(graph, type, type)),
      ).toThrow("Incompatible");
    expect(() =>
      addAdjacentNode(
        graph,
        { ...from, port: "unknown" },
        "right",
        newNode(graph, "core.no_op", "Next"),
      ),
    ).toThrow("Incompatible");
  });
  it("places a node beside a terminal without inventing an output connection", () => {
    const graph = fixture();
    const terminal = newNode(graph, "core.stop", "Stop");
    graph.nodes.push(terminal);
    const next = addAdjacentNode(
      graph,
      { node: terminal.id, port: "", index: 0 },
      "down",
      newNode(graph, "core.no_op", "Next"),
    );
    expect(next.nodes).toHaveLength(graph.nodes.length + 1);
    expect(next.connections).toEqual(graph.connections);
    expect(next.nodes.at(-1)!.position[1]).toBeGreaterThan(
      terminal.position[1],
    );
  });
  it("routes previously saved downward connections from bottom to top", () => {
    const graph = fixture();
    graph.nodes[0]!.position = [80, 80];
    graph.nodes[1]!.position = [80, 320];
    expect(connectionDirection(graph, graph.connections[0]!)).toBe("down");
    graph.nodes[1]!.position = [400, 80];
    expect(connectionDirection(graph, graph.connections[0]!)).toBe("right");
  });
  it("keeps remaining switch connections on the same branch after removing an earlier case", () => {
    const graph = fixture();
    const branch = newNode(graph, "core.switch", "Switch");
    const one = {
      key: "one",
      condition: {
        combinator: "and",
        rules: [{ left: 1, operator: "eq", right: 1 }],
      },
    };
    const two = { ...one, key: "two" };
    branch.params.cases = [one, two];
    graph.nodes.push(branch);
    graph.connections = [
      {
        from: { node: branch.id, port: "case", index: 0 },
        to: { node: graph.nodes[1]!.id, port: "main", index: 0 },
      },
      {
        from: { node: branch.id, port: "case", index: 1 },
        to: { node: graph.nodes[0]!.id, port: "main", index: 0 },
      },
    ];
    const next = editGraphNode(graph, branch.id, {
      params: { ...branch.params, cases: [two] },
    });
    expect(next.connections).toEqual([
      {
        ...graph.connections[1],
        from: { node: branch.id, port: "case", index: 0 },
      },
    ]);
    expect(graph.connections).toHaveLength(2);
  });
  it("starts blank and makes space through repeated insertions on branched graphs", () => {
    expect(emptyGraph().nodes).toEqual([]);
    let graph = fixture();
    const branch = newNode(graph, "core.if", "Branch");
    const tail = newNode(
      { ...graph, nodes: [...graph.nodes, branch] },
      "core.no_op",
      "Tail",
    );
    graph.nodes.push(branch, tail);
    graph.connections.push({
      from: { node: branch.id, port: "false", index: 0 },
      to: { node: tail.id, port: "main", index: 0 },
    });
    for (let index = 0; index < 6; index++)
      graph = insertNode(graph, 0, newNode(graph, "core.no_op", "Inserted"));
    for (const a of graph.nodes)
      for (const b of graph.nodes)
        if (a.id !== b.id) {
          expect(
            Math.abs(a.position[0] - b.position[0]) >= 224 ||
              Math.abs(a.position[1] - b.position[1]) >= 160,
          ).toBe(true);
        }
    for (const link of graph.connections)
      expect(
        graph.nodes.find((node) => node.id === link.to.node)!.position[0],
      ).toBeGreaterThan(
        graph.nodes.find((node) => node.id === link.from.node)!.position[0],
      );
    // Inline insertion now preserves the user's rows. Explicit global arrange
    // remains independently idempotent, rather than running on every insertion.
    const arranged = arrangeGraph(graph);
    expect(arrangeGraph(arranged)).toEqual(arranged);
  });
  it("keeps an inserted card between existing cards and shifts the target and successors, using measured sizes", () => {
    const graph = fixture();
    graph.nodes[0]!.position = [40, 200]; graph.nodes[1]!.position = [400, 200];
    const tail = newNode(graph, "core.no_op", "Tail"); tail.position = [760, 200];
    graph.nodes.push(tail); graph.connections.push({from:{node:graph.nodes[1]!.id,port:"main",index:0},to:{node:tail.id,port:"main",index:0}});
    const node = newNode(graph, "core.no_op", "Inserted");
    const original = structuredClone(graph);
    const sizes = Object.fromEntries([...graph.nodes,node].map(item=>[item.id,{width:300,height:260}]));
    const next = insertNode(graph,0,node,sizes);
    const position = (id:string) => next.nodes.find(item=>item.id===id)!.position;
    expect(position(original.nodes[0]!.id)).toEqual([40,200]);
    expect(position(node.id)).toEqual([436,200]);
    expect(position(original.nodes[1]!.id)).toEqual([832,200]);
    expect(position(tail.id)).toEqual([1192,200]);
    expect(graph).toEqual(original);
    expect(JSON.parse(JSON.stringify(next))).toEqual(next);
  });
  it("inserts along a downward connection without rotating it into a horizontal layout", () => {
    const graph = fixture(); graph.nodes[0]!.position=[100,40];graph.nodes[1]!.position=[100,300];
    const node=newNode(graph,"core.no_op","Middle"),next=insertNode(graph,0,node);
    expect(next.nodes.find(item=>item.id===node.id)!.position).toEqual([100,296]);
    expect(next.nodes[1]!.position).toEqual([100,552]);
    expect(connectionDirection(next,next.connections[0]!)).toBe("down");
  });
  it("inserts on exactly one branch, preserves sibling connections and does not mutate the original", () => {
    const graph = fixture();
    const branch = newNode(graph, "core.if", "Branch");
    graph.nodes.push(branch);
    graph.connections.push({
      from: { node: branch.id, port: "false", index: 0 },
      to: graph.connections[0]!.to,
    });
    const before = structuredClone(graph);
    const node = newNode(graph, "core.no_op", "Inserted");
    const next = insertNode(graph, 1, node);
    expect(graph).toEqual(before);
    expect(next.connections[0]).toEqual(graph.connections[0]);
    expect(next.connections[1]!.from).toEqual({
      node: branch.id,
      port: "false",
      index: 0,
    });
    expect(next.connections[1]!.to.node).toBe(node.id);
    expect(next.connections[2]!.to).toEqual(graph.connections[1]!.to);
    expect(next.nodes.at(-1)!.params).toEqual({});
  });
  it("refuses ambiguous multi-port insertions and outputless nodes", () => {
    const graph = fixture(),
      link = graph.connections[0]!;
    for (const type of [
      "core.if",
      "core.switch",
      "core.merge",
      "core.trigger.manual",
      "core.stop",
    ])
      expect(insertionPorts(graph, link, type)).toBeUndefined();
    expect(() =>
      insertNode(graph, 0, newNode(graph, "core.if", "Branch")),
    ).toThrow("Incompatible insertion");
    expect(() =>
      insertNode(graph, 99, newNode(graph, "core.no_op", "Gone")),
    ).toThrow("no longer exists");
  });
  it("rejects cycles, duplicate links and invalid ports while permitting valid branches", () => {
    const graph = fixture(),
      link = graph.connections[0]!;
    expect(canConnect(graph, link.from, link.to)).toBe(false);
    expect(
      canConnect(
        graph,
        { index: 0, port: "main", node: link.from.node },
        { index: 0, port: "main", node: link.to.node },
      ),
    ).toBe(false);
    const other = newNode(graph, "core.no_op", "Other");
    graph.nodes.push(other);
    const target = { node: other.id, port: "main", index: 0 };
    expect(canConnect(graph, link.to, target)).toBe(true);
    graph.connections.push({ from: link.to, to: target });
    expect(canConnect(graph, target, link.to)).toBe(false);
    expect(canConnect(graph, { ...link.from, port: "unknown" }, target)).toBe(
      false,
    );
  });
  it("uses the existing switch and merge indexed-port contract", () => {
    const graph = fixture();
    const branch = newNode(graph, "core.switch", "Switch");
    expect(ports(branch, "outputs")).toEqual([
      { port: "case", type: "main", index: 0 },
      { port: "fallback", type: "main" },
    ]);
    const merge = newNode(graph, "core.merge", "Merge");
    expect(ports(merge, "inputs")).toEqual([
      { port: "input", type: "main", index: 0 },
      { port: "input", type: "main", index: 1 },
    ]);
  });
});
