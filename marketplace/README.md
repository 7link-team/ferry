# Ferry official templates

Source layout (see `docs/应用市场与集群模板设计.md` §5):

```
apps/<key>/package.json          package metadata (localized name/description, category, topology, drivers)
apps/<key>/icon.svg|png          optional, ≤ 64 KiB
apps/<key>/<semver>/blueprint.json  ferry Application (or, from P2, Blueprint)
apps/<key>/<semver>/parameters.json
apps/<key>/<semver>/files/<name>    optional config file templates
apps/<key>/<semver>/README.md
apps/<key>/<semver>/verified.json   optional [{driver, ferry, evidence}]
```

Regenerate the bundle that Ferry synchronizes after any change:

```
node scripts/build-marketplace-index.mjs
```

Add it as a platform source: Platform management → Template sources → provider GitHub,
project `<owner>/ferry`, branch `main`, index path `marketplace/index.json`.
