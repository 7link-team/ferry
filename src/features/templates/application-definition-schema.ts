import type { Path, Row } from "../deployments/manifest/model";

/** Form additions derived from deployments/manifest.rs and blueprint/model.rs.
 * These are draft values, not a replacement for the server's validation/defaults. */
export function definitionShape(path: Path, blueprint: boolean): Row {
  const key = path.at(-1), parent = path.at(-2);
  if (!path.length) return blueprint
    ? { metadata: { name: "" }, roles: [], drivers: {}, endpoints: [], update: {}, allowSingleHost: false }
    : { metadata: { name: "" }, services: [], strategy: {}, overrides: {} };
  if (key === "metadata") return { name: "", labels: {} };
  if (typeof key === "number") {
    if (parent === "roles") return { name: "", identity: "stateless", instances: { min: 1, max: 1, default: 1 }, service: { image: "" }, ports: [], placement: {}, failover: {}, hooks: {} };
    if (parent === "services") return { name: "", image: "", artifact: { source: "pipeline", pipeline: "", path: "" }, replicas: 1, ports: [], env: [], volumes: [], files: [], command: [], dependsOn: [], health: {}, resources: {}, placement: {}, endpointMode: "vip" };
    if (parent === "ports") return { ...(blueprint ? { name: "" } : {}), container: 8080, published: 8080, protocol: "tcp", ...(!blueprint ? { mode: "ingress" } : {}) };
    if (parent === "env") return { name: "", value: "", valueFrom: "" };
    if (parent === "files") return { fromTemplate: "", path: "", values: {} };
    if (parent === "volumes") return { name: "", path: "" };
    if (parent === "endpoints") return { name: "", role: "", port: "", kind: "direct" };
  }
  if (key === "service") {
    const { name: _name, ports: _ports, ...fields } = definitionShape(["services", 0], false);
    return fields;
  }
  if (key === "instances") return { min: 1, max: 1, default: 1, odd: false };
  if (key === "drivers") return Object.fromEntries(["native", "docker", "compose", "stack", "k8s"].map(driver => [driver, { supported: false }]));
  if (parent === "drivers") return { supported: false, memberAddressing: "" };
  if (key === "artifact") return { source: "pipeline", pipeline: "", path: "", run: "" };
  if (key === "placement") return path.includes("roles") && !path.includes("service")
    ? { distinctHosts: false, colocateWith: "" } : { constraints: [], maxReplicasPerNode: 1 };
  if (key === "health") return { readiness: { httpGet: { path: "/", port: 8080 } }, liveness: { httpGet: { path: "/", port: 8080 } } };
  if (key === "readiness" || key === "liveness") return { httpGet: { path: "/", port: 8080 }, periodSeconds: 10, timeoutSeconds: 1, failureThreshold: 3 };
  if (key === "httpGet") return { path: "/", port: 8080 };
  if (key === "resources") return { requests: {}, limits: {} };
  if (key === "requests" || key === "limits") return { cpu: "", memory: "" };
  if (key === "failover") return { replace: "unsupported" };
  if (key === "hooks" && path.includes("roles")) return { join: [], leave: [], healthy: [] };
  if (key === "update") return { maxParallel: 1, minHealthySeconds: 10, healthTimeout: 180, autoRevert: true };
  if (key === "strategy" && !blueprint) return { type: "recreate", healthTimeout: 120, maxUnavailable: 1 };
  if (key === "overrides") return Object.fromEntries(["native", "docker", "compose", "stack"].map(driver => [driver, {}]));
  return {};
}

export function definitionItem(path: Path, items: unknown[], blueprint: boolean, mode = ""): unknown {
  const key = path.at(-1);
  if (["command", "dependsOn", "constraints", "join", "leave", "healthy"].includes(String(key))) return "";
  const shape = definitionShape([...path, items.length], blueprint);
  if (key === "services" || key === "roles") {
    let suffix = items.length + 1;
    const prefix = key === "services" ? "service" : "role";
    while (items.some(item => item && typeof item === "object" && "name" in item && item.name === `${prefix}-${suffix}`)) suffix++;
    return key === "services" ? { name: `${prefix}-${suffix}`, ...(mode === "native" ? { artifact: { source: "pipeline", pipeline: "", path: "" } } : { image: "" }) }
      : { name: `${prefix}-${suffix}`, identity: "stateless", instances: { min: 1, max: 1, default: 1 }, service: { image: "" } };
  }
  if (Object.keys(shape).length) {
    // Mutually exclusive environment sources are added explicitly, not together.
    if (key === "env") return { name: "", value: "" };
    return shape;
  }
  return items.length && (items[0] === null || typeof items[0] !== "object") ? typeof items[0] === "number" ? 0 : typeof items[0] === "boolean" ? false : "" : {};
}

export function definitionChoices(path: Path): string[] | undefined {
  const key = path.at(-1);
  if (key === "identity") return ["stable", "stateless"];
  if (key === "protocol") return ["tcp", "udp"];
  if (key === "replace") return ["supported", "unsupported"];
  if (key === "endpointMode") return ["vip", "dnsrr"];
  if (key === "mode" && path.includes("ports")) return ["ingress", "host"];
  if (key === "kind" && path.includes("endpoints")) return ["discovery", "vip", "direct"];
  if (key === "type" && path.includes("strategy")) return ["recreate", "rolling"];
  if (key === "source" && path.includes("artifact")) return ["pipeline"];
  if (path.length === 1 && key === "apiVersion") return ["ferry/v1", "ferry/v2"];
  return undefined;
}
