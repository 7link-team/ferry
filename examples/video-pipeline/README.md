# 可检出的视频流水线示例

这是调用外部 LTX 公共服务的实验适配器，不是 Ferry 内置视频节点。源码、提示词、工具版本与流水线一起交付；不再依赖测试人员预先放置的本地脚本或 FFmpeg。公共服务会休眠、排队或变更接口，执行成功不代表服务可用性承诺，也不保证相同种子得到相同视频。

## 环境与边界

- 目标：Windows x86_64 和 Linux x86_64。其他系统/架构明确失败。
- Runner 主机预装 Python 3.10+，命令名为 `python`；Linux 可使用激活后的 Python 虚拟环境。Python 仅使用标准库，不需要 pip 包。
- 主机有 Git，以及 Ferry Actions host 执行所需的 `sh`；Windows 使用 Git for Windows 提供的 shell。不是声明 `self-hosted` 就自动具备这些依赖。
- 运行时从 npm registry 下载锁定的媒体工具包，逐包校验 SHA-512；Windows/Linux 使用各自清单，不要求预装 FFmpeg。`prepare` 每次下载，不暗用旧缓存。
- 外部网络：`registry.npmjs.org`、`VIDEO_BACKEND` 指向的 HTTPS 服务。当前示例只实现该 LTX Gradio `text_to_video` 接口，不能直接换成任意视频服务。
- `VIDEO_BACKEND` 是环境配置，提示词为 `prompt.txt`，所有工具和输出位置都从脚本所在的检出目录推导，不使用宿主机盘符。
- 工具来源、版本、包完整性值见 `tools.lock.json`；包内媒体工具的许可证以相应上游发行包为准。本示例不向源码仓库添加二进制。

## 在 Ferry 中配置

1. 将这个目录作为 `examples/video-pipeline/` 纳入自己的 Git 仓库并提交。当前项目工作区文件尚未提交/发布，不能假定 GitHub 远端已经包含它们。
2. 新建构建流水线，导入 `workflow.yml`。将 `generate` 任务绑定到满足上述环境要求的 Runner，执行镜像选择 `host`。
3. 为 `source` checkout 步骤选择仓库连接、仓库地址和**完整提交 SHA**，检出路径留空（工作区根目录）。Ferry 将此配置保存在 `checkout_bindings`，不是 YAML 中隐含的本机路径。私有仓库使用既有凭据机制。
4. 查看并确认 `VIDEO_BACKEND` 后触发新执行。不要覆盖旧实验执行记录。
5. 执行顺序：检出源码 → 下载并验证工具 → 请求真实生成 → 规范化视频 → 独立探测和完整解码 → 上传文件及证据。

标准 GitHub Actions 使用时将该文件放进 `.github/workflows/`，`actions/checkout` 使用当前仓库；Ferry 的 Runner 绑定与 checkout 绑定是平台配置，不能据此宣称 GitHub 托管环境已经验证。

## 本地命令

从检出仓库根目录执行（先设置 `VIDEO_BACKEND`）：

```text
python examples/video-pipeline/video.py prepare
python examples/video-pipeline/video.py generate
python examples/video-pipeline/video.py verify
python -m unittest discover -s examples/video-pipeline -p test_video.py
```

输出在 `examples/video-pipeline/output/video/`：`short-film.mp4`、后端原片、`result.json` 和转换日志。Ferry 上传得到 `video.zip`，解压后查看 MP4；ZIP 本身不是视频格式。`result.json` 记录源提交 SHA、平台、Python/媒体工具版本及哈希、请求参数、后端事件、媒体探测和最终文件哈希。

验收必须同时满足：真实后端生成、H.264、1280×720、30 fps、150 帧、时长 5.000 秒、无音轨、完整解码成功、下载 ZIP 内 MP4 哈希与执行证据一致。后端失败/超时、工具缺失/损坏、媒体不达标会令步骤失败，不上传成功产物；生成阶段失败后单独上传 `video-diagnostics` 诊断包（存在 `result.json` 时），不当作成功的视频交付。没有自动重试或用模拟视频替代真实生成。

## 验证状态

Windows 的新检出流水线实测结果与 Linux 验证缺口记录于 `docs/ferry-vs-lobehub-real-scenarios-2026-11-12.md`。单元测试中的 Linux 平台选择检查不等于 Linux 原生运行验证。
