---
documentation_revision: "1.0"
organized_on: "2026-11-22"
source_updated_on: "2026-11-23"
---

# Documentation

Start here to find the canonical document for a topic. Directory and file names use English; existing document bodies retain their original language. Documentation describes a mixture of product contracts, proposals, operating instructions and historical work. A design, schema, screenshot or checked task is not evidence that the current deployment implements it.

## Start here

- [0.10.10-alpha release notes](releases/v0.10.10-alpha.md): accessible page headings and usable host preview tables; includes mobile release packaging.
- [0.10.9-alpha release notes](releases/v0.10.9-alpha.md): workspace timezones, workflow diagnostics, mobile release assets and responsive list corrections; verify publication on GitHub.
- [0.10.8-alpha release attempt](releases/v0.10.8-alpha.md): workspace business time, workflow diagnostics and mobile release assets; publication status must be checked on the actual release.
- [0.10.7-alpha release notes](releases/v0.10.7-alpha.md): upgrade compatibility and distribution boundaries.

- [Platform product and technical design](architecture/platform-product-and-technical-design.md): authoritative resource, list and form contracts.
- [Compose deployment guide](guides/compose-quickstart.md): installation and operation.
- [Workflow platform architecture](architecture/workflow-platform-v3.md), [requirements](prd/workflow-platform-prd.md), [data model](prd/workflow-platform-data-model.md), [review](prd/workflow-platform-design-review.md) and [machine-readable contract guide](prd/schemas/README.md).
- [Agent knowledge requirements](prd/agent-knowledge-learning-and-collaboration-prd.md), [data/API](prd/agent-knowledge-data-and-api.md), [localized copy](prd/agent-knowledge-localized-copy.md) and [acceptance review](prd/agent-knowledge-review-and-acceptance.md). These remain proposals with their original approval gates.
- [Client completion plan](plans/2026-11-21-mobile-client-completion.md): the existing implementation and acceptance plan, not a release announcement.

## Organization and version rules

| Directory | Purpose |
| --- | --- |
| `architecture/` | Cross-cutting product and technical contracts |
| `guides/` | Deployment, integration and operation instructions |
| `design/` | Feature designs with their original scope and status |
| `prd/` | Requirements, data/API models, reviews, prototypes and schema validators |
| `plans/` | Implementation plans and acceptance scope |
| `research/` | Comparisons, source analysis and proposals |
| `reference/` | Capability matrices, third-party catalogs and attribution |
| `archive/` | Historical task records, validation evidence and handoffs |
| `releases/` | Versioned release notes and upgrade compatibility |
| `screenshots/` | Existing product screenshots referenced by documents |

Keep one canonical path for a document. Use lowercase English kebab-case filenames, except conventional `README.md` and retained artifact names. Date-specific plans and evidence use `YYYY-MM-DD-...`; stable contracts keep stable filenames. Keep prototypes, screenshots and validators beside the documents that own them.

Every Markdown document has three metadata fields:

- `documentation_revision`: `1.0` is the initial catalogued documentation revision introduced by this organization pass. It does not replace versions such as Pipeline v2, Workflow v3 or a PRD revision declared in the body, and it is not the application version.
- `organized_on`: the date of this structural organization pass.
- `source_updated_on`: the last committed source date before the organization pass, taken from Git. It does not claim the document has been validated against the deployed application. Advance it when the content is substantively updated.

Related documents share entries here; independent requirements, schemas and reviews retain their separate contracts. No byte-identical duplicates were found in the original 164 tracked documentation assets, so none were deleted merely for sharing a topic. Historical implementation tasks were moved to the archive rather than presented as current operating instructions. Original decisions, pending approvals, code samples and evidence remain available.

## Validation boundary

All local Markdown targets were checked after reorganization. The legacy [design contract validator](prd/schemas/README.md) has an existing fixed node-count assertion that fails against today's shared node catalog, both before and after this change. Its assertion is retained; it is not reported as passing. The separate 32-case storage index reference validator passes. Documentation organization does not resolve that historical design/runtime mismatch or announce a new release.

## Supporting assets

- [Workflow prototypes](prd/prototypes/) and [responsive screen images](prd/images/) retain their original asset relationships.
- [Product screenshots](screenshots/) retain their existing English filenames.
- [Third-party node catalog](reference/n8n-2.45.7/) retains its version and original catalog/CSV assets.
- [Dated Codeup capture](archive/evidence/2026-11-02-184819-codeup.txt) is a historical raw capture, preserved without rewriting its contents.

## Catalog

The revision below is the documentation revision, not an application release. Dates refer to original source changes. New organization metadata does not change implementation or acceptance status.

### Architecture

| Document | Revision | Source updated |
| --- | --- | --- |
| [architecture/pipeline-v2.md](architecture/pipeline-v2.md) | 1.0 | 2026-11-02 |
| [architecture/platform-product-and-technical-design.md](architecture/platform-product-and-technical-design.md) | 1.1 | 2026-11-22 |
| [architecture/workflow-platform-v3.md](architecture/workflow-platform-v3.md) | 1.0 | 2026-11-05 |

### Guides

| Document | Revision | Source updated |
| --- | --- | --- |
| [guides/agent-runtime-provisioning.md](guides/agent-runtime-provisioning.md) | 1.0 | 2026-11-11 |
| [guides/compose-quickstart.md](guides/compose-quickstart.md) | 1.0 | 2026-11-19 |
| [guides/compose-sandbox.md](guides/compose-sandbox.md) | 1.0 | 2026-11-19 |
| [guides/deployments-and-transfers.md](guides/deployments-and-transfers.md) | 1.0 | 2026-11-02 |
| [guides/repository-connections.md](guides/repository-connections.md) | 1.0 | 2026-11-02 |
| [guides/workflow-git-capabilities.md](guides/workflow-git-capabilities.md) | 1.0 | 2026-11-09 |

### Feature designs

| Document | Revision | Source updated |
| --- | --- | --- |
| [design/account-otp-two-factor-authentication.md](design/account-otp-two-factor-authentication.md) | 1.0 | 2026-11-10 |
| [design/agent-platform-tools-design.md](design/agent-platform-tools-design.md) | 1.0 | 2026-11-10 |
| [design/ai-model-cli-boundaries.md](design/ai-model-cli-boundaries.md) | 1.0 | 2026-11-14 |
| [design/cluster-orchestration-and-self-healing.md](design/cluster-orchestration-and-self-healing.md) | 1.0 | 2026-11-13 |
| [design/code-insights-design.md](design/code-insights-design.md) | 1.0 | 2026-11-07 |
| [design/conversation-members-and-identity-avatars.md](design/conversation-members-and-identity-avatars.md) | 1.0 | 2026-11-11 |
| [design/host-uploads-and-brand-icons.md](design/host-uploads-and-brand-icons.md) | 1.0 | 2026-11-10 |
| [design/insight-reports-code-reviews-design.md](design/insight-reports-code-reviews-design.md) | 1.0 | 2026-11-04 |
| [design/marketplace-and-cluster-templates.md](design/marketplace-and-cluster-templates.md) | 1.0 | 2026-11-12 |
| [design/mobile-navigation.md](design/mobile-navigation.md) | 1.0 | 2026-11-11 |
| [design/monitoring-and-application-health-phase-1.md](design/monitoring-and-application-health-phase-1.md) | 1.0 | 2026-11-11 |
| [design/preview-automation-design.md](design/preview-automation-design.md) | 1.0 | 2026-11-10 |
| [design/preview-holes-design.md](design/preview-holes-design.md) | 1.0 | 2026-11-21 |
| [design/read-only-role-permissions.md](design/read-only-role-permissions.md) | 1.0 | 2026-11-11 |
| [design/remote-desktop-feasibility.md](design/remote-desktop-feasibility.md) | 1.0 | 2026-11-09 |
| [design/site-settings-design.md](design/site-settings-design.md) | 1.0 | 2026-11-10 |
| [design/template-creation-and-yaml-review.md](design/template-creation-and-yaml-review.md) | 1.0 | 2026-11-13 |
| [design/unified-notifications.md](design/unified-notifications.md) | 1.0 | 2026-11-10 |

### Product requirements and supporting contracts

| Document | Revision | Source updated |
| --- | --- | --- |
| [prd/agent-knowledge-data-and-api.md](prd/agent-knowledge-data-and-api.md) | 1.0 | 2026-11-15 |
| [prd/agent-knowledge-learning-and-collaboration-prd.md](prd/agent-knowledge-learning-and-collaboration-prd.md) | 1.0 | 2026-11-15 |
| [prd/agent-knowledge-localized-copy.md](prd/agent-knowledge-localized-copy.md) | 1.0 | 2026-11-15 |
| [prd/agent-knowledge-review-and-acceptance.md](prd/agent-knowledge-review-and-acceptance.md) | 1.0 | 2026-11-15 |
| [prd/application-ha-topology-and-deployment-drivers.md](prd/application-ha-topology-and-deployment-drivers.md) | 1.0 | 2026-11-14 |
| [prd/database-monitoring-table-statistics-and-wait-diagnostics.md](prd/database-monitoring-table-statistics-and-wait-diagnostics.md) | 1.0 | 2026-11-08 |
| [prd/email-template-editor.md](prd/email-template-editor.md) | 1.0 | 2026-11-09 |
| [prd/native-artifact-application-examples.md](prd/native-artifact-application-examples.md) | 1.0 | 2026-11-08 |
| [prd/nats-three-member-application-template.md](prd/nats-three-member-application-template.md) | 1.0 | 2026-11-08 |
| [prd/platform-file-catalog-preview-and-provenance.md](prd/platform-file-catalog-preview-and-provenance.md) | 1.0 | 2026-11-09 |
| [prd/prototypes/RESPONSIVE.md](prd/prototypes/RESPONSIVE.md) | 1.0 | 2026-11-03 |
| [prd/pxc-three-node-deployment-and-recovery.md](prd/pxc-three-node-deployment-and-recovery.md) | 1.0 | 2026-11-08 |
| [prd/redis-sentinel-application-template.md](prd/redis-sentinel-application-template.md) | 1.0 | 2026-11-08 |
| [prd/repository-connection-scenario-presets.md](prd/repository-connection-scenario-presets.md) | 1.0 | 2026-11-08 |
| [prd/schemas/README.md](prd/schemas/README.md) | 1.0 | 2026-11-05 |
| [prd/workflow-platform-data-model.md](prd/workflow-platform-data-model.md) | 1.0 | 2026-11-05 |
| [prd/workflow-platform-design-review.md](prd/workflow-platform-design-review.md) | 1.0 | 2026-11-05 |
| [prd/workflow-platform-prd.md](prd/workflow-platform-prd.md) | 1.0 | 2026-11-14 |

### Implementation plans

| Document | Revision | Source updated |
| --- | --- | --- |
| [plans/2026-11-11-marketplace-p1-plan.md](plans/2026-11-11-marketplace-p1-plan.md) | 1.0 | 2026-11-11 |
| [plans/2026-11-11-marketplace-p2a-plan.md](plans/2026-11-11-marketplace-p2a-plan.md) | 1.0 | 2026-11-11 |
| [plans/2026-11-12-existing-capability-hardening.md](plans/2026-11-12-existing-capability-hardening.md) | 1.0 | 2026-11-13 |
| [plans/2026-11-12-marketplace-p2b-plan.md](plans/2026-11-12-marketplace-p2b-plan.md) | 1.0 | 2026-11-12 |
| [plans/2026-11-12-orchestration-p2c-plan.md](plans/2026-11-12-orchestration-p2c-plan.md) | 1.0 | 2026-11-12 |
| [plans/2026-11-12-orchestration-p3-plan.md](plans/2026-11-12-orchestration-p3-plan.md) | 1.0 | 2026-11-13 |
| [plans/2026-11-21-mobile-client-completion.md](plans/2026-11-21-mobile-client-completion.md) | 1.0 | 2026-11-21 |
| [plans/ai-agent-conversation-migration.md](plans/ai-agent-conversation-migration.md) | 1.0 | 2026-11-08 |
| [plans/contribution-analytics-implementation.md](plans/contribution-analytics-implementation.md) | 1.0 | 2026-11-02 |
| [plans/lobehub-adoption.md](plans/lobehub-adoption.md) | 1.0 | 2026-11-15 |
| [plans/t16-runner-lifecycle.md](plans/t16-runner-lifecycle.md) | 1.0 | 2026-10-28 |
| [plans/t17-desktop-unattended-updates.md](plans/t17-desktop-unattended-updates.md) | 1.0 | 2026-10-28 |

### Research and proposals

| Document | Revision | Source updated |
| --- | --- | --- |
| [research/2026-11-11-lobehub-source-analysis.md](research/2026-11-11-lobehub-source-analysis.md) | 1.0 | 2026-11-15 |
| [research/2026-11-12-ferry-vs-lobehub-n8n-comfyui.md](research/2026-11-12-ferry-vs-lobehub-n8n-comfyui.md) | 1.0 | 2026-11-15 |
| [research/2026-11-12-ferry-vs-lobehub-real-scenarios.md](research/2026-11-12-ferry-vs-lobehub-real-scenarios.md) | 1.0 | 2026-11-15 |
| [research/contribution-analytics-proposal.md](research/contribution-analytics-proposal.md) | 1.0 | 2026-11-03 |

### References

| Document | Revision | Source updated |
| --- | --- | --- |
| [reference/avatar-artwork-notices.md](reference/avatar-artwork-notices.md) | 1.0 | 2026-11-14 |
| [reference/n8n-2.45.7/README.md](reference/n8n-2.45.7/README.md) | 1.0 | 2026-11-05 |
| [reference/pipeline-feature-benchmark.md](reference/pipeline-feature-benchmark.md) | 1.0 | 2026-11-01 |
| [reference/workflow-capability-matrix.md](reference/workflow-capability-matrix.md) | 1.0 | 2026-11-05 |

### Historical archive

| Document | Revision | Source updated |
| --- | --- | --- |
| [archive/handoffs/2026-11-09-preview-runner-handoff.md](archive/handoffs/2026-11-09-preview-runner-handoff.md) | 1.0 | 2026-11-09 |
| [archive/implementation/t1-repositories.md](archive/implementation/t1-repositories.md) | 1.0 | 2026-10-25 |
| [archive/implementation/t10-functional-completeness.md](archive/implementation/t10-functional-completeness.md) | 1.0 | 2026-10-31 |
| [archive/implementation/t11-templates.md](archive/implementation/t11-templates.md) | 1.0 | 2026-11-09 |
| [archive/implementation/t12-product-completion.md](archive/implementation/t12-product-completion.md) | 1.0 | 2026-11-07 |
| [archive/implementation/t13-connected-builds.md](archive/implementation/t13-connected-builds.md) | 1.0 | 2026-10-27 |
| [archive/implementation/t14-actions-pipelines.md](archive/implementation/t14-actions-pipelines.md) | 1.0 | 2026-10-29 |
| [archive/implementation/t15-code-connections.md](archive/implementation/t15-code-connections.md) | 1.0 | 2026-10-31 |
| [archive/implementation/t2-pipelines.md](archive/implementation/t2-pipelines.md) | 1.0 | 2026-10-25 |
| [archive/implementation/t3a-snowflake.md](archive/implementation/t3a-snowflake.md) | 1.0 | 2026-10-25 |
| [archive/implementation/t3b-bigint-ids.md](archive/implementation/t3b-bigint-ids.md) | 1.0 | 2026-10-25 |
| [archive/implementation/t3c-seaorm.md](archive/implementation/t3c-seaorm.md) | 1.0 | 2026-10-25 |
| [archive/implementation/t3d-schema-builder.md](archive/implementation/t3d-schema-builder.md) | 1.0 | 2026-10-25 |
| [archive/implementation/t4-execution-engine.md](archive/implementation/t4-execution-engine.md) | 1.0 | 2026-10-25 |
| [archive/implementation/t5-build-ui.md](archive/implementation/t5-build-ui.md) | 1.0 | 2026-10-25 |
| [archive/implementation/t6-ci-release.md](archive/implementation/t6-ci-release.md) | 1.0 | 2026-10-25 |
| [archive/implementation/t7-compose-deployments.md](archive/implementation/t7-compose-deployments.md) | 1.0 | 2026-11-09 |
| [archive/implementation/t8-variables.md](archive/implementation/t8-variables.md) | 1.0 | 2026-10-26 |
| [archive/implementation/t9-first-run-initialization.md](archive/implementation/t9-first-run-initialization.md) | 1.0 | 2026-10-26 |
| [archive/validation/cloudflared-validation.md](archive/validation/cloudflared-validation.md) | 1.0 | 2026-11-02 |
