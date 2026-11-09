# 主机远程桌面与截图可行性评估

日期：2026-11-04。状态：方案调研，未选型、未实现、未部署。

## 需求与结论

目标：Ferry 服务端随包提供远程控制服务，HostAgent 随包提供被控能力，浏览器提供控制端及截图，覆盖 Windows、Linux、macOS。这里的远控默认指接管当前用户桌面，接近 RustDesk/AnyDesk；RDP 创建或切换桌面会话属于另一种能力。

可行，但没有经本次调研证实的、一个依赖即可满足全部三平台无人值守/锁屏/提权控制的轻量 SDK。应区分“一个安装入口、内部多个受管进程”和“全部静态编进现有两个二进制”；前者可落地，后者显著提高改造与维护成本。

推荐先验证 MeshCentral/MeshAgent 的成套随包方案；如果严格要求不带 Node.js、尽量复用 Ferry 通道，则验证 noVNC + 平台被控组件。两条路线都必须做三平台原型，不能以项目宣称支持操作系统代替实际能力验收。

## 当前架构核对

- `crates/protocol/src/lib.rs`：HostAgent 主动建立控制连接；按需独立会话连接承载二进制数据。目前只有终端、命令、文件、监控等会话，没有桌面会话。
- `crates/runner/src/session.rs`：按会话分派主机操作，适合扩展截图与桌面能力，但需要新增协议和能力协商，不是直接把 RFB 数据塞进现有终端。
- `crates/server/src/hosts/channel.rs` 与 `hosts/runner/transport/channel.rs`：既有 HostChannel、主机身份及集群通道应复用，避免绕过空间授权或另造主机身份入口。
- `crates/server/src/hosts/remote.rs`：现有主机操作采用空间授权和流式转发模式，可作为桌面端点接入依据。
- `src/features/hosts/HostWorkspace.tsx`：当前工具为 terminal/files，可扩展桌面入口。
- Windows 同时存在系统服务与用户模式。系统服务运行成功不等于能读取交互桌面，截图也有相同限制。
- 根 `Cargo.toml` 当前声明 `UNLICENSED`；组件集成前需核对所选版本及依赖许可证，不能因为代码公开就默认 GPL/AGPL 与产品分发方式兼容。

## 方案清单

推荐分数为本项目需求适配判断，满分 5 分，不是性能实测评分。轻量同时考虑部署、运行依赖、开发和维护成本；本次未测二进制体积、内存、延迟或带宽。

| 方案 | 推荐 | 主要优势 | 主要劣势与理由 |
|---|---:|---|---|
| MeshCentral + MeshAgent 随包运行 | 4.5/5 | 成套提供浏览器远控、跨 NAT 主动回连、Windows/Linux/macOS agent；主项目和 agent README 均声明 Apache-2.0；最接近当前屏幕远控需求 | Node.js 服务与第二 agent 增加体积和维护；不是可直接链接的 Rust SDK。需处理 Ferry 主机与远控节点绑定、登录/授权、升级、断线撤权；三平台细节仍需实测 |
| noVNC + 随包 VNC 被控组件 + Ferry 中继 | 4/5 | noVNC 是可嵌入浏览器 RFB 客户端，主要 MPL-2.0；可沿用 Ferry 主动回连，服务端无需因此增加完整远控管理平台；Web 当前帧截图有 API | noVNC 只解决控制端。三平台捕获、输入、锁屏及安装仍由具体 VNC 实现承担；没有统一现成被控 SDK。常见 VNC 服务端有 GPL 条件；不能把 noVNC 的许可证当作整套许可证 |
| Apache Guacamole + guacd + RDP/VNC | 3.5/5 | 成熟浏览器网关，Apache-2.0，适合运维桌面会话；可复用现有 RDP/VNC 服务 | 需 guacd 和原生依赖；完整应用还有 Java 层。只取 JS 客户端与 guacd 可减少一层，但 Ferry 要自己实现网关对接。仍需目标 RDP/VNC；RDP 不保证同屏，Windows 家庭版等有宿主限制；HostAgent 回连隧道需另接 |
| RustDesk OSS 深度集成 | 3/5 | 原生端远控能力成熟、Rust 技术栈接近，具备会合/中继设计，三平台覆盖 | 客户端和 OSS 服务端为 AGPL-3.0；整体客户端不是轻量嵌入 SDK。官方自托管 Web 控制端属于 Server Pro 部署选项，不能默认由 OSS 服务端一并提供；公开托管 Web 页也不等于可随包自托管 |
| IronRDP + WebAssembly/Web 控件 | 3/5 | Rust 模块化 RDP 实现，MIT OR Apache-2.0；有 WebAssembly/Web 控件，适合研究 Rust 内置 RDP 网关 | 不是现成三平台屏幕捕获与输入引擎；连接已有 RDP 与实现自己的被控桌面服务差别很大。若用其服务端骨架，仍需自己完成 OS 桌面后端与会话生命周期 |
| 自研采集/输入 + WebRTC | 2.5/5（首期） | 可完全融入 HostAgent，能按需优化视频体验与协议 | 平台采集、输入注入、用户会话、编码、拥塞、ICE/TURN、断线恢复都需要工程投入。WebRTC 不是被控桌面 SDK；TURN 也可能产生中继流量。适合长期有专门投入时考虑 |

Deskreen/Weylus 更偏屏幕共享或平板输入，不作为本项目无人值守远控首选。ToDesk/AnyDesk 不按可直接集成的开源引擎评估。

## 三平台边界

| 环境 | 普通桌面与截图 | 无人值守、锁屏及特殊情况 |
|---|---|---|
| Windows | 成熟被控实现通常可覆盖；自研时要处理 DPI、多显示器和活动用户会话 | 服务在 Session 0，需成熟组件或受控的交互会话 helper；UAC 安全桌面、锁屏、多用户切换逐项实测。RDP 是否可作为宿主受 Windows 版本等条件影响 |
| Linux X11 | VNC、截图与输入路径较成熟 | 必须有可访问的显示会话和合适权限；共享当前显示与启动虚拟桌面不是同一功能 |
| Linux Wayland | 支持随 GNOME/KDE/wlroots、版本、portal/PipeWire 后端变化 | 无人值守、登录界面、输入注入限制更明显；不能承诺一个通用 agent 在所有发行版自动获得权限。MeshAgent README 对部分场景给出关闭 Wayland 的建议，不可自动视为全面支持 |
| macOS | 可覆盖，需屏幕录制与辅助功能授权；输入监听等权限按实现核对 | 普通安装不能静默绕过 TCC；企业 MDM 可按策略管理部分权限。锁屏、注销和 FileVault 重启前环境必须区分；FileVault 预启动阶段不应承诺普通 agent 可控制 |
| 无 GUI/无可用显示的主机 | 没有可直接截图和接管的当前桌面 | 继续用终端；如果要额外创建虚拟桌面，需要显式安装与配置，不是截图功能附带能力 |

“支持 Win/Linux/macOS”应在产品上表达为能力探测结果，而非根据操作系统名称直接显示可用。能力至少区分截图、只读观看、输入控制、屏幕选择、授权缺失/无显示/无用户会话。

## 截图方案

1. 当前远控画面截图：优先使用所选远控引擎的当前帧。noVNC 官方提供 `RFB.getImageData()`。图像是已接收帧，不等于主机刚刚采集的全分辨率新帧；未连桌面、帧未就绪时不适用。
2. 主机独立截图：HostAgent 在正确的图形用户会话中按需采集，回传 PNG/JPEG。Rust 候选 XCap（Apache 许可），但不是无人值守 SDK；其 README 的 Wayland 标记含义为“可用但部分特殊场景未完全支持”，仍需锁版本、逐环境实测。
3. 仅截图不需要安装完整远控平台；也不需要用户打开远控页。但权限、图形会话、屏幕枚举和错误反馈仍必须处理。不能通过浏览器 `getDisplayMedia()` 截取远端主机，它采集的是浏览器所在设备。
4. 建议独立截图权限；默认预览/下载，不默认长期保存。若后续需要归档，再明确保存位置、期限及访问规则。

## 推荐接入路线

### 路线 A：最快获得成套跨平台能力

Ferry 安装包管理 MeshCentral 服务进程（含 Node 运行时），HostAgent 安装流程管理 MeshAgent 组件，Ferry Web 对接其浏览器桌面能力。对用户保持一个主机和一个安装入口，但内部并非一个进程。

首个原型先验证 Windows、macOS、Linux X11，随后单独验证指定 GNOME/KDE Wayland 环境。现有终端、文件和命令继续走原通道。不要仅用管理员共享账号 iframe 就宣称完成集成；必须验证空间/主机权限映射、会话票据、撤权与节点删除后访问失效。纯 iframe 原型可用于观察能力，不能代替正式授权设计。

### 路线 B：更贴近 Ferry 的轻量内置方式

浏览器 noVNC → Ferry 鉴权与专用二进制 WebSocket 中继 → HostAgent 主动回连会话 → 随包受管 VNC 组件/本机屏幕。

需要扩展 HostChannel/共享协议及 agent 能力协商，兼容旧 agent；VNC 只绑定受限本地接口，不对公网开放 5900，不暴露任意 TCP 转发。可按需拉起桌面组件并在会话结束后回收，是否支持无人值守必须由平台组件决定。

优点是可沿用既有回连，首期不必引入 NAT 打洞与 TURN。代价是桌面流量经过 Ferry：估算总出口约为并发数 × 每会话实际码率，并考虑服务端入流与出流。先测静态办公与窗口拖动/视频场景，再决定是否需要 WebRTC/P2P。

### 分阶段验收建议

- P0 选型原型：固定目标 OS/桌面环境和浏览器，验证两条候选路线的安装体积、空闲/控制时 CPU/内存、1080p 延迟与带宽、桌面权限、许可证及是否依赖用户手工配置。
- P1 独立截图：屏幕枚举、PNG 下载、无桌面/缺权限/断线明确反馈；确认用户会话隔离、多屏与 HiDPI。
- P2 基本远控：只读/控制权限分离、键鼠、缩放/全屏、当前帧截图、断线释放按键、主动结束、会话审计及旧 agent 禁用提示。
- P3 平台深化：锁屏/UAC/用户切换、Wayland、剪贴板等按需求逐项通过真实机器验收。音频、录像、文件传输不因选了远控引擎自动加入首期。

不对未实测原型给确定工期或 RustDesk 等价能力承诺。主要投入在平台会话、授权和分发升级，而不只是增加一个 Web 标签页。

## MeshCentral 资源占用补充

用户追问轻量程度及内存/CPU。结论：相对完整远程管理平台可称较轻，但相对当前 Ferry 增加的是 Node.js 服务与每机 MeshAgent，不是零成本库。

本次读取官方归档安装指南，包含共享单 CPU、0.6GB RAM 的旧版云主机演示。该演示基于旧系统，只证明历史上可在小实例运行，不是当前版本最低配置、进程 RSS 或并发容量指标。MeshAgent README 未提供统一可信的空闲/远控 CPU 与内存基准，不能将经验数字当作官方实测。

面向少量设备、1–2路远控原型，建议先给远控服务额外预留512MB–1GB内存及1 vCPU量级容量；每台被控主机可先预留50–100MB内存作为原型规划预算。这些均为待实测修正的资源预留建议，不是运行实测、最低要求、上限或承诺；服务端预留不含现有Ferry/操作系统或额外外部数据库。

CPU 无统一百分比：空闲主要维持连接；活跃被控端承担采集/压缩，服务端承担连接、TLS和中继，浏览器承担解码/渲染。画面变化、分辨率、多屏、帧率、平台实现和并发都会改变开销；服务端是否处理图像取决于具体功能路径。原型应记录固定环境下空闲/1080p办公/连续拖动窗口的RSS、CPU（明确整机还是单核）、带宽和响应延迟，再判断是否适合默认随包启用。

来源：[官方归档安装指南](https://meshcentral.com/docs/MeshCentral2InstallGuide.pdf)，README 的 Archive PDFs 入口可核对。

## 官方证据与调研边界

以下项目仓库/官方文档于本次调研实际读取，属于持续变化的主分支资料；确定实现时需固定版本再次核对。

- [MeshCentral README：成套管理、嵌入文档入口、Apache-2.0](https://github.com/Ylianst/MeshCentral/blob/master/readme.md)
- [MeshAgent README：平台、配置、Wayland 提示、Apache-2.0](https://github.com/Ylianst/MeshAgent/blob/master/readme.md)
- [noVNC README：浏览器能力、主要 MPL-2.0、嵌入/API 入口](https://github.com/novnc/noVNC/blob/master/README.md)
- [noVNC API：RFB.getImageData](https://github.com/novnc/noVNC/blob/master/docs/API.md)
- [Guacamole server：RDP/VNC 及原生依赖](https://github.com/apache/guacamole-server/blob/main/README)
- [Guacamole client 许可证](https://github.com/apache/guacamole-client/blob/main/LICENSE)
- [RustDesk 客户端与架构](https://github.com/rustdesk/rustdesk)
- [RustDesk AGPL 许可证](https://github.com/rustdesk/rustdesk/blob/master/LICENCE)
- [RustDesk OSS server：hbbs/hbbr 与 Pro 区别](https://github.com/rustdesk/rustdesk-server)
- [RustDesk 官方 Web V2 部署说明源文：公开 Web 与自托管 Pro](https://github.com/rustdesk/doc.rustdesk.com/blob/master/v3/src/content/post/en/web-client-v2-preview.mdx)
- [IronRDP：Rust、WebAssembly、客户端与服务器骨架](https://github.com/Devolutions/IronRDP)
- [LibVNCServer：GPL 与链接说明](https://github.com/LibVNC/libvncserver/blob/master/README.md)
- [XCap：许可证、系统依赖与支持矩阵图例](https://github.com/nashaofu/xcap)

本次只检查架构与上游资料，没有运行任何远控引擎、没有跨系统实机测试，也没有修改业务代码或安装服务。最终选型和产品权限/保存策略尚未确认。
