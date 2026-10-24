# T6 — 把新 harness 接进 CI，并把发布链路准备好

## 必读

- `.github/workflows/build.yml`、`server-image.yml`、`.github/scripts/smoke-server-image.mjs`。
- `scripts/test-backends.sh`（已在 CI 里，看它怎么接的）、`test-ssh.sh`、`test-runner.sh`、`test-pipelines.sh`、`test-build-ui.mjs`。
- `CLAUDE.md`。

## 背景

现在 CI 只跑 `test-backends.sh` 和基本的 check。这几个真正验证端到端行为的 harness 都只在本地跑过：

| harness | 验证什么 | 现在在 CI 里吗 |
|---|---|---|
| `test-ssh.sh` | SSH 通道（真 sshd 容器） | ❌ |
| `test-runner.sh` | runner 通道（真二进制） | ❌ |
| `test-pipelines.sh` | 执行引擎（真 Git + 两种主机） | ❌ |
| `test-build-ui.mjs` | 界面真实链路（Playwright） | ❌ |

本地跑过不等于别人改了代码之后还会跑。接进 CI 才算有门禁。

## 1. 接进 CI

给 `build.yml` 加 job，跑在 `ubuntu-24.04`（Docker 可用）。可以合成一个 job 分步跑，也可以分成几个并行 job——你判断哪种在失败时更容易定位，并说明理由。

几条硬要求：

- **不能静默跳过。** 这些 harness 里有几个在缺少依赖时会跳过（例如 `FERRY_REQUIRE_LIVE`、`FERRY_REQUIRE_ALL`）。CI 里必须把这些开关都打开，缺依赖直接失败。缺了开关的 harness 要补一个等价机制。
- **跑了几项检查要可见。** 已有的 harness 会打印 `N passed`；如果某个 harness 没有最少检查数下限，补上。CI 里「0 个检查跑过」必须算失败——本仓库出现过脚本在 0 个测试匹配时打印「全部通过」。
- `release` job 的 `needs` 要加上新 job，否则发版可能带着没验证过的代码出去。
- 用 `Swatinem/rust-cache@v2`，复用现有的 key 习惯。Playwright 浏览器要缓存或显式安装，不要每次重装拖慢。

## 2. 验证不是 flaky

这是本任务最重要的一条。**每个新 job 至少连续跑 3 次全绿**才算完成。

本仓库已经吃过两次亏：`test-runner.sh` 的清理竞态让它约 1/4 概率失败；macOS 的 DMG 打包偶发挂载失败。两次都是「本地跑一次就过」给的假信心。

跑法由你定（`gh workflow run` 触发几次、或在一个 job 里循环几遍），但报告里要给出**实际的 3 次结果**，不是「应该稳定」。

发现 flaky 就修根因，不要加重试掩盖——除非你能证明是外部依赖的间歇故障（像 hdiutil 那次），那时重试要带明确的「三次都失败就是真失败」的日志。

## 3. 前端分包

`npm run build` 一直有 chunk 超过 500KB 的警告。xterm、React、Radix 和业务代码现在全在一个包里，首屏要把终端库一起下载，而大多数页面用不到它。

用 Vite 的 `manualChunks` 拆开，至少把 xterm 单独分出来（它只在主机详情的终端标签页用到，那个标签页本来就是按需挂载的）。

目标是消除警告而不是把阈值调高。如果某个包确实无法拆到阈值以下，说明理由。

## 4. 发布链路

**不要推 tag，也不要创建 release。** 那是对外动作，由我决定时机和版本号。

你要做的是让它**可被验证**：

- 确认 `release` job 的触发条件、`needs`、产物收集在加了新 job 之后仍然正确。
- 如果有办法在不真正发布的前提下验证（例如用 `workflow_dispatch` 跑一次只到产物收集为止的路径，或在 PR 上用 `act` 之类），做一次并给出结果。做不到就说明为什么，以及推 tag 时需要注意什么。

## 验收门槛

```sh
actionlint .github/workflows/*.yml
npm run build          # 无 chunk 超限警告
npm run typecheck && npm test
cargo fmt --all -- --check && cargo check --workspace --all-targets && cargo test --workspace
```

加上：每个新 CI job 的 **3 次连续运行结果**。

## 不要做的事

- 不推 tag、不建 release、不改仓库可见性或包可见性。
- 不为了让 CI 变绿而放宽 harness 的断言或降低检查数。
- 不 `git commit` / `git push`——我来提交；但你可以用 `gh workflow run` 触发已经在远端的 workflow 做验证，前提是不推新代码。

> 注意：新 workflow 要在远端生效才能被触发。如果验证必须先推代码，**停下来告诉我**，由我推送后你再验证，不要自己推。

## 交回

报告：改动文件、新 job 的结构与理由、每个 job 的 3 次运行结果（含 run id）、分包前后的产物大小、发布链路的验证方式与结论、以及任何按边界没改的问题。
