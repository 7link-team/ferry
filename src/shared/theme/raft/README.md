# Raft theme adapter

The `neoBrutalism` palette uses [Raft Design System](https://github.com/botiverse/raft-design), pinned to commit `1656b5a1c25add2ceda52c4e4d50aab99db27835`.

`tokens.json` is the unmodified upstream `packages/ui/src/tokens.json`. Run `node scripts/build-raft-theme.mjs` to regenerate the scoped, prefixed `tokens.generated.css`; `--check` verifies it without writing or using the network. Do not edit generated CSS. `theme.css` maps those tokens to Ferry's existing semantic roles and adapts existing controls without introducing another component framework.

The adapter follows upstream foundations for color, dark mode, shape, elevation, states and typography. Yellow belongs to navigation and the outer frame; pink marks primary actions. Content rests on opaque paper surfaces. Controls use 1.5px ink outlines and 8px corners; cards use 10px, dialogs 16px. Controls stay flat and stationary; transient overlays use the official soft floating shadow.

Code, tokens and documentation retain the upstream [MIT license](./LICENSE). Locally bundled Space Grotesk and Space Mono fonts retain their [SIL Open Font License](./fonts/OFL.txt). The upstream [notice](./NOTICE.md) is preserved. No Raft logo, name as a deployment brand, avatars or reaction artwork is included.
