# T9 Platform initialization and management
## Goal / Requirements
First installation requires a human-created platform superadmin. No default credentials. Provide platform users, workspaces and registration settings. Support existing installations without silently promoting accounts.
## Existing Implementation / Reuse
Accounts/sessions, workspace permissions and shared resource table/form DSL. Extend global resource endpoint configuration; retain workspace scoping.
## Task Decomposition / Execution Mode
Direct: schema and setup → platform API → UI/desktop → real regression verification.
## Design
Append users is_superadmin/version and singleton platform_state. Operator token file gates one-time transactional setup. Existing-account promotion requires its password. Desktop consumes local token through fragment. Global admin routes use /_platform and /api/platform. Serialize privileged mutations and protect last active admin. Revoke sessions on privilege/status change. Reuse activity with workspace_id=0 for platform audit, never place secrets in audit records.
## Files / Boundaries
Server auth, workspace, resource, migration/entities, new platform module; Entry/App/router/shared DSL and new platform page; desktop server handoff; fixture scripts. Do not change T8 variables or published migrations.
## Testing / Acceptance
Setup success/failure/replay/concurrency/legacy proof; admin access and tenant isolation; last admin and stale mutations; registration gate; browser setup and platform forms; Rust and frontend checks. Existing deployment workflows retained.
## Risks / Out of Scope
No default password, account password reset, workspace deletion or variable retention changes. Alpha release only after verification.
