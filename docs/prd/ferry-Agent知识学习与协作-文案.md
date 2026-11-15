# Agent 知识、学习与协作：三语文案

版本v1.0-review，2026-11-11。所有key拟用前缀 `agentKnowledge.`；仅文档，不写入运行时语言包。中文是本PRD新增拟定源文案，英文/马来文为对应拟定译文，等待R07母语校对与真实UI验证；不是已有官方翻译。通用分页、保存、编辑、返回、刷新、必填等复用现有共享组件语言键，实施时逐项核对，不重复创建同义键。

占位符使用命名参数，不拼接语序；`{count}` 使用语言复数规则，`{max}` 为已格式化数值，`{time}` 按用户已选时区显示并附IANA标识。用户正文与工具结果保持原文，不当系统文案翻译。状态键按对象命名映射，不能由后端英文枚举直接显示。下表列出新增功能中的名称、字段、类型、动作、状态、提示和错误；实现不得临时加入无三语覆盖的字符串。

## 1. 名称、字段与操作

| Key | zh-CN | en-US | ms-MY |
|---|---|---|---|
| memories | 记忆 | Memories | Ingatan |
| skills | 技能 | Skills | Kemahiran |
| learning | 学习与复盘 | Learning and review | Pembelajaran dan semakan |
| collaboration | 协作任务 | Collaboration task | Tugasan kerjasama |
| context | 上下文组成 | Context details | Butiran konteks |
| evidence | 来源证据 | Source evidence | Bukti sumber |
| proposals | 待审候选 | Proposals for review | Cadangan untuk semakan |
| receipts | 应用回执 | Application receipts | Rekod pelaksanaan perubahan |
| branches | 会话分支 | Conversation branches | Cabang perbualan |
| dispatch | 派发状态 | Dispatch status | Status penghantaran tugasan |
| title | 标题 | Title | Tajuk |
| body | 正文 | Content | Kandungan |
| kind | 类型 | Type | Jenis |
| fact | 事实 | Fact | Fakta |
| preference | 偏好 | Preference | Keutamaan |
| contextType | 持续上下文 | Ongoing context | Konteks berterusan |
| manual | 手动输入 | Manual entry | Entri manual |
| message | 消息 | Message | Mesej |
| turn | 执行回合 | Execution turn | Giliran pelaksanaan |
| owner | 所有者 | Owner | Pemilik |
| visibility | 可见范围 | Visibility | Keterlihatan |
| private | 仅自己 | Only me | Saya sahaja |
| workspace | 工作区共享 | Shared with workspace | Dikongsi dengan ruang kerja |
| boundAgent | 绑定 Agent | Assigned agent | Ejen yang ditetapkan |
| noAgent | 不绑定 Agent | No assigned agent | Tiada ejen ditetapkan |
| expires | 到期时间 | Expiry time | Masa tamat tempoh |
| noExpiry | 不设到期时间 | No expiry | Tiada tarikh tamat tempoh |
| locked | 人工锁定 | Locked by user | Dikunci oleh pengguna |
| sourceVersion | 来源版本 | Source version | Versi sumber |
| version | 版本 | Version | Versi |
| skillName | 技能名称 | Skill name | Nama kemahiran |
| candidateVersion | 候选版本 | Candidate version | Versi calon |
| publishedVersion | 已发布版本 | Published version | Versi diterbitkan |
| compare | 比较版本 | Compare versions | Bandingkan versi |
| applicability | 适用条件 | When to use | Syarat penggunaan |
| steps | 操作步骤 | Steps | Langkah |
| verification | 验证方法 | Verification | Pengesahan |
| failureHandling | 失败处理 | Failure handling | Pengendalian kegagalan |
| extraction | 自动生成记忆候选 | Generate memory proposals automatically | Jana cadangan ingatan secara automatik |
| dailyReview | 定期复盘 | Scheduled review | Semakan berjadual |
| modelProfile | 模型档案 | Model profile | Profil model |
| embeddingModel | 嵌入模型 | Embedding model | Model pembenaman |
| sourceScope | 来源会话范围 | Source conversations | Perbualan sumber |
| timezone | 时区 | Time zone | Zon waktu |
| schedule | 调度表达式 | Schedule expression | Ungkapan jadual |
| nextRun | 下次运行 | Next run | Pelaksanaan seterusnya |
| budget | 预算来源 | Budget source | Sumber belanjawan |
| goal | 任务目标 | Task goal | Matlamat tugasan |
| coordinator | 负责人 Agent | Coordinator agent | Ejen penyelaras |
| members | 成员 Agent | Member agents | Ejen ahli |
| dependencies | 前置任务 | Dependencies | Tugasan prasyarat |
| mode | 协作模式 | Collaboration mode | Mod kerjasama |
| sequence | 顺序执行 | Sequential | Berurutan |
| parallel | 并行执行 | Parallel | Selari |
| handoff | 交接执行 | Handoff | Serahan tugas |
| plan | 执行计划 | Execution plan | Pelan pelaksanaan |
| deadline | 总期限 | Overall time limit | Had masa keseluruhan |
| readOnly | 只读 | Read only | Baca sahaja |
| onRequest | 按请求审批 | Approval on request | Kelulusan atas permintaan |
| currentBranch | 当前分支 | Current branch | Cabang semasa |
| forkPoint | 分叉消息 | Branch point | Mesej titik cabang |
| reason | 原因 | Reason | Sebab |
| newMemory | 新建记忆 | New memory | Ingatan baharu |
| saveMemory | 保存为记忆 | Save as memory | Simpan sebagai ingatan |
| activate | 确认生效 | Activate | Aktifkan |
| invalidate | 标记失效 | Mark invalid | Tandakan tidak sah |
| archive | 归档 | Archive | Arkibkan |
| restoreDraft | 恢复为草稿 | Restore as draft | Pulihkan sebagai draf |
| lock | 锁定自动修订 | Lock automatic changes | Kunci perubahan automatik |
| unlock | 解除锁定 | Unlock | Nyahkunci |
| sharedDraft | 新建共享草稿 | Create shared draft | Cipta draf perkongsian |
| publish | 发布此版本 | Publish this version | Terbitkan versi ini |
| disable | 停用 | Disable | Nyahaktifkan |
| enable | 启用 | Enable | Aktifkan |
| restoreRevision | 以历史版本新建候选 | Create proposal from this version | Cipta cadangan daripada versi ini |
| accept | 接受候选 | Accept proposal | Terima cadangan |
| reject | 拒绝候选 | Reject proposal | Tolak cadangan |
| runReview | 立即复盘 | Review now | Semak sekarang |
| approve | 允许本次操作 | Allow this operation | Benarkan operasi ini |
| deny | 拒绝本次操作 | Deny this operation | Tolak operasi ini |
| start | 确认启动 | Confirm and start | Sahkan dan mulakan |
| stop | 请求停止 | Request stop | Minta hentikan |
| createBranch | 从此处分叉 | Branch from here | Cipta cabang dari sini |
| switchBranch | 切换分支 | Switch branch | Tukar cabang |
| showSource | 查看来源 | View source | Lihat sumber |
| showReceipt | 查看回执 | View receipt | Lihat rekod perubahan |

## 2. 状态与结果

| Key | zh-CN | en-US | ms-MY |
|---|---|---|---|
| state.draft | 草稿 | Draft | Draf |
| state.active | 已生效 | Active | Aktif |
| state.invalid | 已失效 | Invalid | Tidak sah |
| state.archived | 已归档 | Archived | Diarkibkan |
| state.enabled | 已启用 | Enabled | Diaktifkan |
| state.disabled | 已停用 | Disabled | Dinyahaktifkan |
| state.pending | 待处理 | Pending | Belum diproses |
| state.applied | 已应用 | Applied | Telah digunakan |
| state.rejected | 已拒绝 | Rejected | Ditolak |
| state.stale | 版本已过期 | Outdated | Versi lapuk |
| state.queued | 排队中 | Queued | Dalam baris gilir |
| state.running | 执行中 | Running | Sedang dilaksanakan |
| state.completed | 已完成 | Completed | Selesai |
| state.failed | 失败 | Failed | Gagal |
| state.cancelling | 正在停止 | Stopping | Sedang dihentikan |
| state.cancelled | 已取消 | Cancelled | Dibatalkan |
| state.unknown | 执行状态未知 | Execution status unknown | Status pelaksanaan tidak diketahui |
| state.ready | 索引就绪 | Index ready | Indeks sedia |
| state.waiting_human | 等待人工处理 | Waiting for a person | Menunggu tindakan pengguna |
| state.summarizing | 汇总中 | Summarizing | Sedang merumuskan |
| state.stopping_unknown | 停止结果待确认 | Stop status unconfirmed | Status penghentian belum disahkan |
| state.dispatched | 已派发 | Dispatched | Telah dihantar |
| state.prepared | 待发送 | Prepared | Sedia untuk dihantar |
| state.sent | 已发送，待确认 | Sent, awaiting confirmation | Dihantar, menunggu pengesahan |
| state.accepted | 本地已接收 | Accepted locally | Diterima secara setempat |
| state.terminal | 执行已结束 | Execution ended | Pelaksanaan tamat |
| state.aborted | 已确认未启动 | Confirmed not started | Disahkan belum bermula |
| state.timed_out | 已超时 | Timed out | Tamat masa |
| state.unrecognized | 状态无法识别，请刷新 | Unrecognized status. Refresh to continue. | Status tidak dikenali. Muat semula untuk meneruskan. |
| result.noop | 无需变更 | No changes needed | Tiada perubahan diperlukan |
| result.unverified | 未验证 | Unverified | Belum disahkan |
| result.verified | 已验证 | Verified | Disahkan |
| result.deleted | 内容已删除 | Content deleted | Kandungan dipadamkan |

## 3. 提示、确认与异常

| Key | zh-CN | en-US | ms-MY |
|---|---|---|---|
| empty.memories | 暂无记忆，可手动创建或从有权访问的消息保存。 | No memories yet. Create one or save an accessible message. | Belum ada ingatan. Cipta ingatan atau simpan mesej yang boleh anda akses. |
| empty.skills | 暂无技能，先创建草稿再审阅发布。 | No skills yet. Create a draft, then review and publish it. | Belum ada kemahiran. Cipta draf, kemudian semak dan terbitkannya. |
| empty.proposals | 暂无待审候选。 | No proposals to review. | Tiada cadangan untuk disemak. |
| empty.context | 本次未注入记忆或技能。 | No memories or skills were added to this turn. | Tiada ingatan atau kemahiran ditambahkan pada giliran ini. |
| empty.search | 未找到可用记忆。 | No available memories found. | Tiada ingatan yang tersedia ditemukan. |
| hint.reference | 记忆是参考资料，不授予执行权限。 | Memories are reference material and do not grant execution permissions. | Ingatan ialah bahan rujukan dan tidak memberikan kebenaran pelaksanaan. |
| hint.candidate | 自动生成的内容须审阅后才能生效。 | Generated content requires review before it takes effect. | Kandungan yang dijana perlu disemak sebelum berkuat kuasa. |
| hint.skillAccept | 接受候选只创建技能草稿，不会自动发布。 | Accepting creates a skill draft and does not publish it. | Penerimaan hanya mencipta draf kemahiran dan tidak menerbitkannya. |
| hint.shared | 共享须创建独立草稿，禁止直接公开私聊衍生内容。 | Sharing requires a separate draft. Private-chat derivatives cannot be published directly. | Perkongsian memerlukan draf berasingan. Kandungan terbitan sembang peribadi tidak boleh diterbitkan terus. |
| hint.timezone | 检测到时区 {timezone}，请选择并确认。 | Detected time zone: {timezone}. Select and confirm a time zone. | Zon waktu dikesan: {timezone}. Pilih dan sahkan zon waktu. |
| hint.nextRun | 下次运行：{time} | Next run: {time} | Pelaksanaan seterusnya: {time} |
| hint.partial | 仅搜索了部分内容，请缩小范围。 | Only part of the content was searched. Narrow the scope. | Hanya sebahagian kandungan dicari. Kecilkan skop carian. |
| hint.degraded | 语义检索不可用，当前仅使用关键词。 | Semantic search is unavailable. Keyword search is in use. | Carian semantik tidak tersedia. Carian kata kunci sedang digunakan. |
| hint.truncated | 部分内容因上下文预算未加入。 | Some content was omitted due to the context budget. | Sebahagian kandungan tidak disertakan kerana had konteks. |
| hint.nativeHistory | 无法撤回已发送给原生 Agent 的内容，将建立新会话。 | Content already sent to the native agent cannot be recalled. A new session is required. | Kandungan yang telah dihantar kepada ejen asal tidak boleh ditarik balik. Sesi baharu diperlukan. |
| confirm.publish | 发布后，新执行将使用此版本。正在执行的任务保留原版本。 | New executions will use this version. Running tasks retain their original version. | Pelaksanaan baharu akan menggunakan versi ini. Tugasan yang sedang berjalan mengekalkan versi asal. |
| confirm.shared | 我确认此草稿不包含无权共享的私聊、凭证或第三方受限内容。 | I confirm this draft contains no private-chat content, credentials, or restricted third-party content I am not permitted to share. | Saya mengesahkan draf ini tidak mengandungi kandungan sembang peribadi, kelayakan akses atau kandungan pihak ketiga terhad yang saya tidak dibenarkan kongsi. |
| confirm.delete | 删除后将停止提供正文，资产按引用规则清理。 | Content will become unavailable. Assets will be cleaned up according to reference rules. | Kandungan tidak lagi tersedia. Aset akan dibersihkan mengikut peraturan rujukan. |
| confirm.stop | 将停止后续派发并请求终止活动任务，已发生的外部操作不会自动撤销。 | Further dispatches will stop and active tasks will receive a stop request. External actions already performed will not be undone automatically. | Penghantaran seterusnya akan dihentikan dan tugasan aktif akan menerima permintaan berhenti. Tindakan luaran yang telah dilakukan tidak akan dibatalkan secara automatik. |
| error.sourceUnavailable | 来源已变化或不可访问，请重新核对内容。 | The source changed or is inaccessible. Review the content again. | Sumber telah berubah atau tidak boleh diakses. Semak kandungan semula. |
| error.conflict | 内容已被修改，请刷新后重新审阅。 | The content changed. Refresh and review it again. | Kandungan telah berubah. Muat semula dan semak semula. |
| error.idempotency | 此请求标识已用于其他内容。 | This request identifier was used for different content. | Pengecam permintaan ini telah digunakan untuk kandungan lain. |
| error.locked | 内容已人工锁定，不能自动修订。 | This content is locked against automatic changes. | Kandungan ini dikunci daripada perubahan automatik. |
| error.activeReferences | 仍有活动任务引用，暂不能删除。 | Active tasks still reference this content. It cannot be deleted yet. | Tugasan aktif masih merujuk kandungan ini. Kandungan belum boleh dipadamkan. |
| error.privateShare | 私聊衍生内容不能直接共享。 | Private-chat derivatives cannot be shared directly. | Kandungan terbitan sembang peribadi tidak boleh dikongsi terus. |
| error.invalidTransition | 当前状态不允许此操作。 | This operation is not allowed in the current state. | Operasi ini tidak dibenarkan dalam status semasa. |
| error.limit | 不得超过 {max}。 | Must not exceed {max}. | Tidak boleh melebihi {max}. |
| error.skillName | 使用小写字母、数字和连字符，并以字母或数字开头。 | Use lowercase letters, digits, and hyphens. Start with a letter or digit. | Gunakan huruf kecil, angka dan sempang. Mulakan dengan huruf atau angka. |
| error.timezone | 请选择有效的 IANA 时区。 | Select a valid IANA time zone. | Pilih zon waktu IANA yang sah. |
| error.model | 模型未配置、未获授权或不支持此能力。 | The model is unconfigured, unauthorized, or does not support this capability. | Model belum dikonfigurasikan, tidak dibenarkan atau tidak menyokong keupayaan ini. |
| error.budget | 预算不足或计费能力不可用，未启动模型调用。 | The budget is insufficient or billing is unavailable. No model call was started. | Belanjawan tidak mencukupi atau pengebilan tidak tersedia. Tiada panggilan model dimulakan. |
| error.cycle | 任务依赖形成循环，请修改计划。 | Task dependencies contain a cycle. Edit the plan. | Kebergantungan tugasan membentuk kitaran. Ubah pelan. |
| error.readOnlyUnsupported | 此运行环境无法确认只读限制，不能启动并行征询。 | This runtime cannot guarantee read-only restrictions. Parallel consultation cannot start. | Persekitaran ini tidak dapat mengesahkan sekatan baca sahaja. Rundingan selari tidak boleh dimulakan. |
| error.permissionChanged | 权限已变化，任务正在停止。 | Permissions changed. The task is stopping. | Kebenaran telah berubah. Tugasan sedang dihentikan. |
| error.onlyInitiator | 仅发起人可审批本次操作。 | Only the initiator can approve this operation. | Hanya pemula boleh meluluskan operasi ini. |
| error.dispatchUnknown | 无法确认任务是否已启动，禁止重复派发。 | Task startup is unconfirmed. Repeated dispatch is blocked. | Permulaan tugasan belum disahkan. Penghantaran berulang disekat. |
| error.binding | 原生会话绑定不兼容，不能继续使用。 | The native session binding is incompatible and cannot be resumed. | Pengikatan sesi asal tidak serasi dan sesi tidak boleh disambung semula. |
| error.runnerUpgrade | 当前 Runner 不支持此能力，请先升级。 | This Runner does not support this capability. Upgrade it first. | Runner ini tidak menyokong keupayaan ini. Naik taraf dahulu. |
| error.branchHead | 分支已新增消息，请刷新后发送。 | New messages were added to this branch. Refresh before sending. | Mesej baharu telah ditambahkan pada cabang ini. Muat semula sebelum menghantar. |
| error.roundLimit | 已达到协作轮次上限，任务未全部完成。 | The collaboration round limit was reached. Some work remains incomplete. | Had pusingan kerjasama telah dicapai. Sebahagian kerja belum selesai. |
| error.unknownState | 状态未定义，已禁止修改，请刷新或联系管理员。 | The state is undefined. Changes are blocked. Refresh or contact an administrator. | Status tidak ditakrifkan. Perubahan disekat. Muat semula atau hubungi pentadbir. |

## 4. 文案验收

检查每key三语非空、占位符集合一致、状态表覆盖全部公开枚举。确认删除/停止/共享的完整句子必须在Phone显示全文，不能缩成无解释的“确定”。网络/加载/分页/通用字段校验复用项目公共i18n，不泄露供应商正文或源对象名称。新错误代码在落地时必须绑定这里的语义键；落地注册尚未执行。

R07保留：母语校对及ms-MY最长文案三设备、200%字体截图。本文提供翻译草案，不冒充语言专家终审。
