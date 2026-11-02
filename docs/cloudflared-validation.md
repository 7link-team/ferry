# Cloudflare Tunnel live validation

`scripts/test-cloudflared.py` exercises a real credentialless quick tunnel,
an isolated Ferry server and a real Runner. It does not use or upgrade an
existing deployment. Named tunnels and account-specific access rules require
separate credentials and are outside this quick-tunnel check.

Build the server and Runner first, then run:

```sh
cargo build --locked -p ferry-server -p ferry-runner
python scripts/test-cloudflared.py
```

Set `CLOUDFLARED_BINARY`, `FERRY_BINARY` or `FERRY_RUNNER_BINARY` when the
executables are outside the default locations. The test uses HTTP/2 to work on
networks that block outbound QUIC; `CLOUDFLARED_PROTOCOL=quic` tests QUIC explicitly.
For cloudflared versions that provide the startup connectivity precheck,
`CLOUDFLARED_NO_PRECHECKS=1` bypasses that diagnostic. It does not bypass Ferry
authentication, Host validation, TLS verification or any acceptance assertion.

The harness creates its own data directory and ephemeral origin port. Once the
quick-tunnel hostname is assigned, it adds that exact hostname to the isolated
server's `FERRY_ALLOWED_HOSTS` and restarts the origin. Public deployments need
the same allowlist configuration; the harness does not rewrite the Host header
or relax the server's request guard.

Acceptance requires:

- Public HTTPS returns the isolated server's status.
- Public WSS rejects an invalid protocol handshake.
- A real Runner enrolls over WSS and successfully executes a connection test.
- After the origin restarts, the same Runner reconnects using its saved identity
  and successfully executes another connection test. A persisted `online` value
  alone is insufficient evidence of reconnection.
- All owned processes terminate and the origin port closes.

Logs and a machine-readable result remain under
`data/maintenance/cloudflared-<unique>/`. This live external-service test is
opt-in, so routine pull-request checks do not depend on quick-tunnel availability.

Validated on Windows with cloudflared 2026.7.3 using HTTP/2 and
`CLOUDFLARED_NO_PRECHECKS=1`: all acceptance checks passed. This environment
blocked QUIC, and cloudflared's startup precheck did not advance to tunnel
registration; bypassing that diagnostic allowed HTTP/2 registration. Initial
test attempts also caught missing public-host allowlisting and a reconnection
readiness race in the harness. Both were corrected while preserving all server
security checks. These results do not establish named-tunnel or QUIC support in
this environment.
