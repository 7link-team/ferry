# Notice

Vendored from [botiverse/raft-design](https://github.com/botiverse/raft-design/tree/1656b5a1c25add2ceda52c4e4d50aab99db27835).
This directory contains its unmodified `packages/ui/src/tokens.json`, local font files
and a semantic theme adapter. Fonts and their OFL notice are in [`fonts/`](./fonts/).
The upstream notice below refers to paths in the upstream repository. No Botiverse
logos, avatars or reaction artwork are included.

## Code and documentation

The code and documentation in this repository are licensed under the MIT License. See [`LICENSE`](./LICENSE).

## Fonts

`design/fonts/` contains Space Grotesk and Space Mono. Both are licensed under the SIL Open Font License, Version 1.1, not under the MIT License. The copyright notices and the license text are in [`design/fonts/OFL.txt`](./design/fonts/OFL.txt).

The `raft-ui` npm package does not include these font files; its `fonts.css` loads the fonts from Google Fonts.

## Brand assets

The following are brand assets of Botiverse:

- the Raft name;
- the Raft logo and mark, in `design/assets/Logos`;
- the agent pixel avatars, in `design/assets/Agents`;
- the reaction artwork, in `design/assets/Reactions`.

Copies and renderings of these assets appear elsewhere in the repository, for example the logo, icons and reaction sprite in `apps/doc/public` and the `RaftLogo` component in `packages/ui`.

The MIT License that covers this repository does not grant rights to the Raft name, logo, mark, agent pixel avatars or reaction artwork. Code that renders them, such as the `RaftLogo` component, is licensed under the MIT License; the assets it renders are not.
