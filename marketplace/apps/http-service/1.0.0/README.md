# HTTP sample service

BusyBox `httpd` serving one page whose text comes from the `message` parameter.

- Port: container `8080`, published on the `port` parameter.
- Stateless: no volumes; safe to delete and recreate.
- Verified by the Ferry CI driver suite for Docker, Compose and Swarm (single host).
