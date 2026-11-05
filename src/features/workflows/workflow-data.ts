import { manifest, ports, type GraphNode } from "./graph";
import type { WorkflowAsset } from "./WorkflowAssets";

export interface WorkflowItem {
  json: Record<string, unknown>;
  binary?: Record<string, WorkflowAsset>;
  paired?: unknown[];
}
export interface WorkflowPacket {
  ports: {
    name: string;
    index: number;
    skipped?: boolean;
    items: WorkflowItem[];
  }[];
}
export interface NodeExecutionData {
  input: {
    items: WorkflowItem[];
    sources: {
      node: string;
      iteration_hash: string;
      port: string;
      index: number;
      items: WorkflowItem[];
    }[];
  };
  output: WorkflowPacket | null;
  resolved_params: Record<string, unknown>[];
  frozen_at: number;
  revision_id: string;
  state: string;
  end_reason: string;
}
export const fieldExpression = (path: (string | number)[]) =>
  `={{ $json${path.map((key) => `[${JSON.stringify(key)}]`).join("")} }}`;
export function parameterSelections(
  node: GraphNode,
  length: number,
): (number | null)[] {
  return manifest(node.type)?.item_mode === "per_item"
    ? Array.from(
        {
          length:
            node.settings?.execute_once === true ? Math.min(length, 1) : length,
        },
        (_, index) => index,
      )
    : [null];
}
export function nodeModes(node: GraphNode) {
  return [
    "active",
    ...(["core.if", "core.switch", "core.merge", "core.stop"].includes(
      node.type,
    )
      ? []
      : ["bypass"]),
    "mute",
  ];
}
export const canPinNode = (node: GraphNode) =>
  ports(node, "outputs").length === 1;
export const successfulNodeStates = [
  "succeeded",
  "skipped",
  "bypassed",
  "cached",
  "pinned",
  "reused",
];

/** Use the browser's normal input event so the existing parameter forms remain authoritative. */
export function insertFieldReference(
  field: HTMLInputElement | HTMLTextAreaElement,
  value: string,
) {
  if (
    field.disabled ||
    field.readOnly ||
    (field instanceof HTMLInputElement &&
      !["text", "search", "url", "email"].includes(field.type))
  )
    return false;
  const prototype =
    field instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(field, value);
  field.dispatchEvent(new Event("input", { bubbles: true }));
  field.focus();
  return true;
}
