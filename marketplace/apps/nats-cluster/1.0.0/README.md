# NATS JetStream cluster

Three (or 5, 7, 9) `nats:2.10-alpine` members forming one JetStream cluster.
Each member is a stable instance `nats-<index>` with its own `nats-<index>-data` volume.

- Placement: one member per host (`distinctHosts`). Compose puts all members on one
  host for demos only (`allowSingleHost`; not highly available).
- Ports: client `4222`, routes `6222`, monitoring `8222` in the container. Instance N
  publishes `client_port + N`, `cluster_port + N` and `monitor_port + N`, so members never
  collide even when hosts share a network namespace.
- Routes: `files/nats.conf` lists every member with `{{ range .Role "nats" }}`. With the
  Docker driver a member is reached at its bound address and published route port; with
  Compose at its service name and container port.
- Readiness: `/healthz?js-enabled-only=true`, so a member becomes ready before the
  cluster has a meta leader and members on later hosts can still join.
- Drivers: Docker (one host per member) and Compose (single host). Native needs a
  pipeline-built `nats-server` artifact and is not offered by this package.
- No authentication or TLS is configured; keep the ports on a private network or add
  them before exposing the cluster.
- Topology changes (adding or removing members) are not supported yet; change
  parameters or the image only.
