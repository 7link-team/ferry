# Ferry official templates

Source layout (see `docs/应用市场与集群模板设计.md` §5):

```
apps/<key>/package.yaml          package metadata (localized name/description, category, topology, drivers) — like Helm Chart.yaml
apps/<key>/icon.svg|png          optional, ≤ 64 KiB
apps/<key>/<semver>/blueprint.yaml  ferry Application or Blueprint — like Helm templates/
apps/<key>/<semver>/parameters.yaml typed parameters — like Nomad Pack variables / Helm values.schema
apps/<key>/<semver>/files/<name>    optional config file templates
apps/<key>/<semver>/README.md
apps/<key>/<semver>/verified.yaml   optional [{driver, ferry, evidence}]
```

Sources are YAML (`.yml` and legacy `.json` are also read; keep one per name). The generated
`index.json` stays JSON because it is the machine format Ferry synchronizes.

Regenerate the bundle that Ferry synchronizes after any change:

```
node scripts/build-marketplace-index.mjs
```

Add it as a platform source: Platform management → Template sources → provider GitHub,
project `<owner>/ferry`, branch `main`, index path `marketplace/index.json`.
