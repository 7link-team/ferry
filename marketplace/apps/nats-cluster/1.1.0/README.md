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
- Drivers: Docker (one host per member), Compose (single host) and Swarm stack. Native
  needs a pipeline-built `nats-server` artifact and is not offered by this package.
- Swarm stack: deployed through the bound manager; each member is its own service pinned
  to its host's Swarm node (`node.id==…`, `dnsrr`) and publishes its ports in host mode,
  so members advertise and route to each other exactly as with the Docker driver. A
  member's update stops its old task before starting the new one (its host ports are
  taken until then).
- No authentication or TLS is configured; keep the ports on a private network or add
  them before exposing the cluster.
- Topology changes (adding or removing members) are not supported yet; change
  parameters or the image only.

## Failure recovery

Stateless roles can be rebuilt on another matching host (class A). This template's
stable NATS members declare replicated replacement (class B): automatic recovery
requires a healthy majority and an available matching host. Roles whose state
cannot be reconstructed from replicas require backups and only raise alerts.

Set `failover.mode` in the deployment spec to `auto` (default), `alert`, or `off`.
`failover.hostDownAfter` defaults to 90 seconds and accepts integers from 30 to
3600. This is the host silence threshold, not a promised recovery duration: probe
evidence, quorum, cooldown and replacement health checks also gate recovery.
Compose and Swarm stateful deployments only alert; automatic member movement is
supported by the Docker driver in this package.

A replacement retains `server_name: nats-<index>` and joins the routes with an empty
local data volume. Recovery depends on data replicas on surviving members; it does
not restore single-copy data or data whose replica quorum has been lost. Use
`alert` mode when that prerequisite is not met.

No `leave` or `join` hook is configured: the package does not depend on the `nats`
CLI. Its healthy hook invokes `wget` against the local JetStream-enabled health
endpoint. [Linux CI](https://github.com/7link-team/ferry/actions/runs/43380131479/job/131735830420)
verified that `nats:2.10-alpine` contains `wget` and no `nats` CLI. Its 97 checks
include killing a fixture host, rebuilding the member on a spare host, retaining
20 messages in an R3 stream with both replicas current, executing the healthy hook,
and removing the old container before clearing its fence when the host returns.
Replication is observed with the read-only `STREAM.INFO` API; repeated
`STREAM.CREATE` responses can contain a follower's local peer view.

The fixtures use independent Docker daemons on one Linux host and share its kernel;
this does not certify physical-host failure isolation. Compose verification covers
single-host deployment, and [Swarm verification](https://github.com/7link-team/ferry/actions/runs/43380131479/job/131735830057)
covers deployment, not automatic stateful member movement.
