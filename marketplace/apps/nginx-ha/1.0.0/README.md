# Nginx HA web

Stateless Nginx web instances (`web-0`, `web-1`, …) behind one or more Nginx load
balancers (`lb-0`, …). Every web instance serves `<message> from <instance>`.

- Roles: `web` (2–10 instances, one per host) and `lb` (1–3 instances, one per host).
  The load balancer's upstream in `files/nginx-lb.conf` lists every web instance with
  `{{ range .Role "web" }}`; an `lb` instance may share a host with a `web` instance.
- Ports: both roles listen on `80` in the container. Web instance N publishes
  `web_port + N`; load balancer instance N publishes `lb_port + N`.
- Addressing: with the Docker driver the balancer reaches web instances at their bound
  address and published port; with Compose at the service name and port `80`.
- Readiness: web `/`, load balancer `/healthz` (answered locally, so a balancer can
  start before its upstreams).
- Drivers: Docker (instances on distinct hosts), Compose (single host; demo only,
  not highly available) and Swarm stack. Native is not offered by this package.
- Swarm stack: `web` and `lb` are each one replicated service (one replica per bound
  instance) publishing `web_port` / `lb_port` on the ingress routing mesh. The balancer
  reaches web replicas through the `web` service VIP on port `80`, and every web replica
  serves `<message> from web`.
- Changing `message` is a parameter upgrade: a new revision replaces every web instance.
