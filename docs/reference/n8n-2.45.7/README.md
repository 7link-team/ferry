# n8n 2.45.7 内置节点实机清单

提取时间：2026-11-04T20:13:20.093Z。来自本机实际安装包及已登录运行实例的节点目录，不采用官网营销数量。

## 数量与口径

| 项目 | 数量 |
|---|---:|
| 安装包内独立节点类型 | 572 |
| n8n-nodes-base（2.45.6） | 445 |
| @n8n/n8n-nodes-langchain（2.45.6） | 127 |
| 其中运行实例加载的包内类型 | 568 |
| 运行实例另外生成的 Tool 变体 | 278 |
| 其中人工审核 Tool 变体（已包含在上一行） | 10 |
| 运行实例提供的 MCP 目录类型 | 78 |
| 运行目录独立类型合计 | 924 |
| 运行目录包含历史版本的描述记录（不当成类型数） | 1003 |
| 运行目录凭据类型 | 527 |
| 包内节点静态 operation 选项记录（见下方限制） | 4101 |
| 其中专用操作选项记录 | 3496 |
| 其中通用 Custom API Call 入口记录 | 605 |

- 一个节点类型不等于一个应用：同一应用可有普通节点、触发节点、模型节点；Tool 变体单列、不计作新增应用。其中人工审核变体只适用发送并等待等审核参数，继承的完整参数描述不代表所有操作都能作为审核工具执行。
- MCP 目录表示连接远端服务的入口；远端工具列表取决于服务与授权，不能按目录项声称内置了全部远端实现。
- 按类型标识去重，资源与操作取默认版本对应的参数描述；保留条件声明，不把旧版与新版操作简单相加。
- 静态操作数是声明的选项记录数，不是已测试能力数；保留 resource、版本及其它显示条件，相同操作名可能属于不同资源。动态加载操作、远端工具、无 operation 选择器的节点不在该数内。
- 运行实例会注入 Custom API Call 通用选项；在数据中标记 genericCustomApi，并与专用操作分开统计，不将这些通用入口视为额外的原生服务操作。
- 隐藏项、弃用项、默认排除项保留并标记；目录存在不等于普通新增节点搜索可见。触发标记依据 group=trigger 或 polling；带 webhook 的普通发送/等待节点不因此算作触发器。类别可重叠。
- 本清单不包括另外安装的社区节点，不表示 Ferry 已兼容或实现这些能力；没有执行各服务的商业账号验收。
- 只输出名称、标识、版本、分类及参数选项等目录事实，不复制节点实现、认证实现、图标、原始参数定义或任何凭据值。

## 文件

- [nodes.csv](nodes.csv)：完整节点表，可用表格筛选应用、类型、分类、隐藏/加载状态及操作数量。
- [operations.csv](operations.csv)：静态操作及事件选项，含资源/显示条件；Tool 变体单独标识。
- [credentials.csv](credentials.csv)：凭据类型及被哪些节点引用，不含密钥。
- [catalog.json](catalog.json)：机器可读的同一份提取结果，含资源、操作、事件、版本与条件。

## 分类统计（包内类型，可重复归类）

| 分类 | 类型数 |
|---|---:|
| AI | 123 |
| Analytics | 13 |
| Communication | 109 |
| Core Nodes | 79 |
| Data & Storage | 62 |
| Developer Tools | 3 |
| Development | 119 |
| ECM | 1 |
| Finance & Accounting | 16 |
| HITL | 10 |
| Marketing | 54 |
| Miscellaneous | 16 |
| Productivity | 51 |
| Sales | 46 |
| Utility | 25 |

## 当前未加载的包内类型

- n8n-nodes-base.dynamicCredentialCheck
- n8n-nodes-base.e2eTest
- n8n-nodes-base.executeCommand
- n8n-nodes-base.localFileTrigger

## 用户提到的示例（默认参数描述）

### Git

类型：`n8n-nodes-base.git`；默认版本：1.1；凭据：gitPassword。

| 资源/显示条件 | 操作 | 标识 |
|---|---|---|
| {} | Add | add |
| {} | Add Config | addConfig |
| {} | Clone | clone |
| {} | Commit | commit |
| {} | Fetch | fetch |
| {} | List Config | listConfig |
| {} | Log | log |
| {} | Pull | pull |
| {} | Push | push |
| {} | Push Tags | pushTags |
| {} | Reflog | reflog |
| {} | Status | status |
| {} | Switch Branch | switchBranch |
| {} | Tag | tag |
| {} | User Setup | userSetup |

### GitHub

类型：`n8n-nodes-base.github`；默认版本：1.1；凭据：githubApi, githubAppApi, githubOAuth2Api。

资源：File (file)、Issue (issue)、Organization (organization)、Pull Request (pullRequest)、Release (release)、Repository (repository)、Review (review)、User (user)、Workflow (workflow)、Custom API Call (__CUSTOM_API_CALL__)。

| 资源/显示条件 | 操作 | 标识 |
|---|---|---|
| {"show":{"resource":["organization"]}} | Get Repositories | getRepositories |
| {"show":{"resource":["organization"]}} | Get Members | getMembers |
| {"show":{"resource":["organization"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["pullRequest"]}} | Create | create |
| {"show":{"resource":["pullRequest"]}} | Update | update |
| {"show":{"resource":["pullRequest"]}} | Close | close |
| {"show":{"resource":["pullRequest"]}} | Reopen | reopen |
| {"show":{"resource":["pullRequest"]}} | Get | get |
| {"show":{"resource":["pullRequest"]}} | Create Comment | createComment |
| {"show":{"resource":["pullRequest"]}} | Edit Comment | editComment |
| {"show":{"resource":["pullRequest"]}} | Get Diff | getDiff |
| {"show":{"resource":["pullRequest"]}} | Get Patch | getPatch |
| {"show":{"resource":["pullRequest"]}} | Merge | merge |
| {"show":{"resource":["pullRequest"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["issue"]}} | Create | create |
| {"show":{"resource":["issue"]}} | Create Comment | createComment |
| {"show":{"resource":["issue"]}} | Edit | edit |
| {"show":{"resource":["issue"]}} | Get | get |
| {"show":{"resource":["issue"]}} | Lock | lock |
| {"show":{"resource":["issue"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["file"]}} | Create | create |
| {"show":{"resource":["file"]}} | Delete | delete |
| {"show":{"resource":["file"]}} | Edit | edit |
| {"show":{"resource":["file"]}} | Get | get |
| {"show":{"resource":["file"]}} | List | list |
| {"show":{"resource":["file"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["repository"]}} | Get | get |
| {"show":{"resource":["repository"]}} | Get Issues | getIssues |
| {"show":{"resource":["repository"]}} | Get License | getLicense |
| {"show":{"resource":["repository"]}} | Get Profile | getProfile |
| {"show":{"resource":["repository"]}} | Get Pull Requests | getPullRequests |
| {"show":{"resource":["repository"]}} | List Popular Paths | listPopularPaths |
| {"show":{"resource":["repository"]}} | List Referrers | listReferrers |
| {"show":{"resource":["repository"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["user"]}} | Get Repositories | getRepositories |
| {"show":{"resource":["user"]}} | Get Issues | getUserIssues |
| {"show":{"resource":["user"]}} | Invite | invite |
| {"show":{"resource":["user"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["release"]}} | Create | create |
| {"show":{"resource":["release"]}} | Delete | delete |
| {"show":{"resource":["release"]}} | Get | get |
| {"show":{"resource":["release"]}} | Get Many | getAll |
| {"show":{"resource":["release"]}} | Update | update |
| {"show":{"resource":["release"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["review"]}} | Create | create |
| {"show":{"resource":["review"]}} | Get | get |
| {"show":{"resource":["review"]}} | Get Many | getAll |
| {"show":{"resource":["review"]}} | Update | update |
| {"show":{"resource":["review"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["workflow"]}} | Disable | disable |
| {"show":{"resource":["workflow"]}} | Dispatch | dispatch |
| {"show":{"resource":["workflow"]}} | Dispatch and Wait for Completion | dispatchAndWait |
| {"show":{"resource":["workflow"]}} | Enable | enable |
| {"show":{"resource":["workflow"]}} | Get | get |
| {"show":{"resource":["workflow"]}} | Get Usage | getUsage |
| {"show":{"resource":["workflow"]}} | List | list |
| {"show":{"resource":["workflow"]}} | Custom API Call | __CUSTOM_API_CALL__ |

### GitLab

类型：`n8n-nodes-base.gitlab`；默认版本：1；凭据：gitlabApi, gitlabOAuth2Api。

资源：File (file)、Issue (issue)、Release (release)、Repository (repository)、User (user)、Custom API Call (__CUSTOM_API_CALL__)。

| 资源/显示条件 | 操作 | 标识 |
|---|---|---|
| {"show":{"resource":["issue"]}} | Create | create |
| {"show":{"resource":["issue"]}} | Create Comment | createComment |
| {"show":{"resource":["issue"]}} | Edit | edit |
| {"show":{"resource":["issue"]}} | Get | get |
| {"show":{"resource":["issue"]}} | Lock | lock |
| {"show":{"resource":["issue"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["repository"]}} | Get | get |
| {"show":{"resource":["repository"]}} | Get Issues | getIssues |
| {"show":{"resource":["repository"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["user"]}} | Get Repositories | getRepositories |
| {"show":{"resource":["user"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["release"]}} | Create | create |
| {"show":{"resource":["release"]}} | Delete | delete |
| {"show":{"resource":["release"]}} | Get | get |
| {"show":{"resource":["release"]}} | Get Many | getAll |
| {"show":{"resource":["release"]}} | Update | update |
| {"show":{"resource":["release"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["file"]}} | Create | create |
| {"show":{"resource":["file"]}} | Delete | delete |
| {"show":{"resource":["file"]}} | Edit | edit |
| {"show":{"resource":["file"]}} | Get | get |
| {"show":{"resource":["file"]}} | List | list |
| {"show":{"resource":["file"]}} | Custom API Call | __CUSTOM_API_CALL__ |

### Slack

类型：`n8n-nodes-base.slack`；默认版本：2.7；凭据：slackApi, slackOAuth2Api。

资源：Channel (channel)、File (file)、Message (message)、Reaction (reaction)、Star (star)、User (user)、User Group (userGroup)。

| 资源/显示条件 | 操作 | 标识 |
|---|---|---|
| {"show":{"resource":["channel"]}} | Archive | archive |
| {"show":{"resource":["channel"]}} | Close | close |
| {"show":{"resource":["channel"]}} | Create | create |
| {"show":{"resource":["channel"]}} | Get | get |
| {"show":{"resource":["channel"]}} | Get Many | getAll |
| {"show":{"resource":["channel"]}} | History | history |
| {"show":{"resource":["channel"]}} | Invite | invite |
| {"show":{"resource":["channel"]}} | Join | join |
| {"show":{"resource":["channel"]}} | Kick | kick |
| {"show":{"resource":["channel"]}} | Leave | leave |
| {"show":{"resource":["channel"]}} | Member | member |
| {"show":{"resource":["channel"]}} | Open | open |
| {"show":{"resource":["channel"]}} | Rename | rename |
| {"show":{"resource":["channel"]}} | Replies | replies |
| {"show":{"resource":["channel"]}} | Set Purpose | setPurpose |
| {"show":{"resource":["channel"]}} | Set Topic | setTopic |
| {"show":{"resource":["channel"]}} | Unarchive | unarchive |
| {"show":{"resource":["message"]}} | Delete | delete |
| {"show":{"resource":["message"]}} | Delete Scheduled | deleteScheduled |
| {"show":{"resource":["message"]}} | Get Many Scheduled | getManyScheduled |
| {"show":{"resource":["message"]}} | Get Permalink | getPermalink |
| {"show":{"resource":["message"]}} | Schedule | schedule |
| {"show":{"resource":["message"]}} | Search | search |
| {"show":{"resource":["message"]}} | Send | post |
| {"show":{"resource":["message"]}} | Send and Wait for Response | sendAndWait |
| {"show":{"resource":["message"]}} | Update | update |
| {"show":{"resource":["star"]}} | Add | add |
| {"show":{"resource":["star"]}} | Delete | delete |
| {"show":{"resource":["star"]}} | Get Many | getAll |
| {"show":{"resource":["file"]}} | Get | get |
| {"show":{"resource":["file"]}} | Get Many | getAll |
| {"show":{"resource":["file"]}} | Upload | upload |
| {"show":{"resource":["reaction"]}} | Add | add |
| {"show":{"resource":["reaction"]}} | Get | get |
| {"show":{"resource":["reaction"]}} | Remove | remove |
| {"show":{"resource":["user"]}} | Get | info |
| {"show":{"resource":["user"]}} | Get Many | getAll |
| {"show":{"resource":["user"]}} | Get User's Profile | getProfile |
| {"show":{"resource":["user"]}} | Get User's Status | getPresence |
| {"show":{"resource":["user"]}} | Look Up by Email | lookupByEmail |
| {"show":{"resource":["user"]}} | Update User's Profile | updateProfile |
| {"show":{"resource":["userGroup"]}} | Add Users | updateUsers |
| {"show":{"resource":["userGroup"]}} | Create | create |
| {"show":{"resource":["userGroup"]}} | Disable | disable |
| {"show":{"resource":["userGroup"]}} | Enable | enable |
| {"show":{"resource":["userGroup"]}} | Get Many | getAll |
| {"show":{"resource":["userGroup"]}} | Get Users | getUsers |
| {"show":{"resource":["userGroup"]}} | Update | update |

### Anthropic

类型：`@n8n/n8n-nodes-langchain.anthropic`；默认版本：1；凭据：anthropicApi。

资源：Document (document)、File (file)、Image (image)、Prompt (prompt)、Text (text)、Custom API Call (__CUSTOM_API_CALL__)。

| 资源/显示条件 | 操作 | 标识 |
|---|---|---|
| {"show":{"resource":["document"]}} | Analyze Document | analyze |
| {"show":{"resource":["document"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["file"]}} | Upload File | upload |
| {"show":{"resource":["file"]}} | Get File Metadata | get |
| {"show":{"resource":["file"]}} | List Files | list |
| {"show":{"resource":["file"]}} | Delete File | deleteFile |
| {"show":{"resource":["file"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["image"]}} | Analyze Image | analyze |
| {"show":{"resource":["image"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["prompt"]}} | Generate Prompt | generate |
| {"show":{"resource":["prompt"]}} | Improve Prompt | improve |
| {"show":{"resource":["prompt"]}} | Templatize Prompt | templatize |
| {"show":{"resource":["prompt"]}} | Custom API Call | __CUSTOM_API_CALL__ |
| {"show":{"resource":["text"]}} | Message a Model | message |
| {"show":{"resource":["text"]}} | Custom API Call | __CUSTOM_API_CALL__ |

### Anthropic Chat Model

类型：`@n8n/n8n-nodes-langchain.lmChatAnthropic`；默认版本：1.6；凭据：anthropicApi。

没有静态 operation 选择器；能力通过节点本身与参数配置提供，不能因此计为零能力。

## 基础包完整节点表（445）

| 名称 | 类型标识 | 版本 | 分类 | 静态操作数 | 状态 |
|---|---|---|---|---:|---|
| Action Network | n8n-nodes-base.actionNetwork | 1 | Marketing, Sales | 30 | 已加载 |
| ActiveCampaign | n8n-nodes-base.activeCampaign | 1 | Marketing | 60 | 已加载 |
| ActiveCampaign Trigger | n8n-nodes-base.activeCampaignTrigger | 1 | Marketing | 0 | 已加载 / 触发 |
| Acuity Scheduling Trigger | n8n-nodes-base.acuitySchedulingTrigger | 1 | Productivity | 0 | 已加载 / 触发 |
| Adalo | n8n-nodes-base.adalo | 1 | Data & Storage | 6 | 已加载 |
| Affinity | n8n-nodes-base.affinity | 1 | Sales | 16 | 已加载 |
| Affinity Trigger | n8n-nodes-base.affinityTrigger | 1 | Sales | 0 | 已加载 / 触发 |
| Aggregate | n8n-nodes-base.aggregate | 1 | Core Nodes | 0 | 已加载 |
| Agile CRM | n8n-nodes-base.agileCrm | 1 | Marketing, Sales | 15 | 已加载 |
| AI Transform | n8n-nodes-base.aiTransform | 1 | Core Nodes, Development | 0 | 已加载 / 隐藏 |
| Airtable | n8n-nodes-base.airtable | 2.2 | Data & Storage | 8 | 已加载 |
| Airtable Trigger | n8n-nodes-base.airtableTrigger | 1 | Data & Storage | 0 | 已加载 / 触发 |
| Airtop | n8n-nodes-base.airtop | 1 | Development, Productivity | 24 | 已加载 |
| AMQP Sender | n8n-nodes-base.amqp | 1 | Communication, Development | 0 | 已加载 |
| AMQP Trigger | n8n-nodes-base.amqpTrigger | 1 | Communication, Development | 0 | 已加载 / 触发 |
| APITemplate.io | n8n-nodes-base.apiTemplateIo | 1 | Marketing | 6 | 已加载 |
| Asana | n8n-nodes-base.asana | 1 | Productivity | 29 | 已加载 |
| Asana Trigger | n8n-nodes-base.asanaTrigger | 1 | Productivity | 0 | 已加载 / 触发 |
| Autopilot | n8n-nodes-base.autopilot | 1 | Marketing | 11 | 已加载 |
| Autopilot Trigger | n8n-nodes-base.autopilotTrigger | 1 | Marketing | 0 | 已加载 / 触发 |
| AWS Certificate Manager | n8n-nodes-base.awsCertificateManager | 1 | Development | 6 | 已加载 |
| AWS Cognito | n8n-nodes-base.awsCognito | 1 | Development | 16 | 已加载 |
| AWS Comprehend | n8n-nodes-base.awsComprehend | 1 | Development | 4 | 已加载 |
| AWS DynamoDB | n8n-nodes-base.awsDynamoDb | 1 | Data & Storage, Development | 5 | 已加载 |
| AWS ELB | n8n-nodes-base.awsElb | 1 | Development | 9 | 已加载 |
| AWS IAM | n8n-nodes-base.awsIam | 1 | Development | 14 | 已加载 |
| AWS Lambda | n8n-nodes-base.awsLambda | 1 | Development | 2 | 已加载 |
| AWS Rekognition | n8n-nodes-base.awsRekognition | 1 | Development | 2 | 已加载 |
| AWS S3 | n8n-nodes-base.awsS3 | 2 | Data & Storage, Development | 15 | 已加载 |
| AWS SES | n8n-nodes-base.awsSes | 1 | Communication, Development | 16 | 已加载 |
| AWS SNS | n8n-nodes-base.awsSns | 1 | Communication, Development | 4 | 已加载 |
| AWS SNS Trigger | n8n-nodes-base.awsSnsTrigger | 1 | Communication, Development | 0 | 已加载 / 触发 |
| AWS SQS | n8n-nodes-base.awsSqs | 1 | Communication, Development | 2 | 已加载 |
| AWS Textract | n8n-nodes-base.awsTextract | 1 | Utility | 2 | 已加载 |
| AWS Transcribe | n8n-nodes-base.awsTranscribe | 1 | Development | 5 | 已加载 |
| Azure Cosmos DB | n8n-nodes-base.azureCosmosDb | 1 | Data & Storage | 12 | 已加载 |
| Azure Storage | n8n-nodes-base.azureStorage | 1 | Data & Storage | 10 | 已加载 |
| BambooHR | n8n-nodes-base.bambooHr | 1 | Miscellaneous | 15 | 已加载 |
| Bannerbear | n8n-nodes-base.bannerbear | 1 | Marketing | 4 | 已加载 |
| Baserow | n8n-nodes-base.baserow | 1.1 | Data & Storage | 9 | 已加载 |
| Beeminder | n8n-nodes-base.beeminder | 1 | Productivity | 22 | 已加载 |
| Bitbucket Trigger | n8n-nodes-base.bitbucketTrigger | 1.1 | Development, Productivity | 0 | 已加载 / 触发 |
| Bitly | n8n-nodes-base.bitly | 1 | Utility | 4 | 已加载 |
| Bitwarden | n8n-nodes-base.bitwarden | 1 | Data & Storage | 19 | 已加载 |
| Box | n8n-nodes-base.box | 1 | Data & Storage | 15 | 已加载 |
| Box Trigger | n8n-nodes-base.boxTrigger | 1 | Data & Storage | 0 | 已加载 / 触发 |
| Brandfetch | n8n-nodes-base.Brandfetch | 2 | Sales, Utility | 4 | 已加载 |
| Brevo | n8n-nodes-base.sendInBlue | 1.1 | Communication, Marketing | 19 | 已加载 |
| Brevo Trigger | n8n-nodes-base.sendInBlueTrigger | 1 | Communication, Marketing | 0 | 已加载 / 触发 |
| Bubble | n8n-nodes-base.bubble | 1 | Development | 5 | 已加载 |
| Cal.com Trigger | n8n-nodes-base.calTrigger | 3 | Productivity, Utility | 0 | 已加载 / 触发 |
| Calendly Trigger | n8n-nodes-base.calendlyTrigger | 2 | Productivity, Utility | 0 | 已加载 / 触发 |
| Chargebee | n8n-nodes-base.chargebee | 1 | Finance & Accounting | 15 | 已加载 |
| Chargebee Trigger | n8n-nodes-base.chargebeeTrigger | 1 | Finance & Accounting | 0 | 已加载 / 触发 |
| Check Credential Status | n8n-nodes-base.dynamicCredentialCheck | 1 |  | 0 | 未加载 |
| CircleCI | n8n-nodes-base.circleCi | 1 | Development | 3 | 已加载 |
| Clearbit | n8n-nodes-base.clearbit | 1 | Sales | 5 | 已加载 |
| ClickUp | n8n-nodes-base.clickUp | 1 | Communication, Productivity | 71 | 已加载 |
| ClickUp Trigger | n8n-nodes-base.clickUpTrigger | 1 | Communication, Productivity | 0 | 已加载 / 触发 |
| Clockify | n8n-nodes-base.clockify | 1 | Productivity | 32 | 已加载 |
| Clockify Trigger | n8n-nodes-base.clockifyTrigger | 1 | Productivity, Utility | 0 | 已加载 / 触发 |
| Cloudflare | n8n-nodes-base.cloudflare | 1 | Development | 5 | 已加载 |
| Cockpit | n8n-nodes-base.cockpit | 1 | Development, Marketing | 5 | 已加载 |
| Coda | n8n-nodes-base.coda | 1.1 | Productivity | 18 | 已加载 |
| Code | n8n-nodes-base.code | 2 | Core Nodes, Development | 0 | 已加载 |
| CoinGecko | n8n-nodes-base.coinGecko | 1 | Finance & Accounting, Productivity | 9 | 已加载 |
| Compare Datasets | n8n-nodes-base.compareDatasets | 2.3 | Core Nodes | 0 | 已加载 |
| Compression | n8n-nodes-base.compression | 1.1 | Core Nodes, Data & Storage | 2 | 已加载 |
| Confluence | n8n-nodes-base.confluence | 1 | Productivity | 22 | 已加载 |
| Contentful | n8n-nodes-base.contentful | 1 | Development, Marketing | 7 | 已加载 |
| Convert to File | n8n-nodes-base.convertToFile | 1.2 | Core Nodes | 10 | 已加载 |
| Convert to/from binary data | n8n-nodes-base.moveBinaryData | 1.1 | Core Nodes | 0 | 已加载 / 隐藏 |
| ConvertKit | n8n-nodes-base.convertKit | 1 | Marketing, Sales | 20 | 已加载 |
| ConvertKit Trigger | n8n-nodes-base.convertKitTrigger | 1 | Marketing, Sales | 0 | 已加载 / 触发 |
| Copper | n8n-nodes-base.copper | 1 | Marketing, Sales | 40 | 已加载 |
| Copper Trigger | n8n-nodes-base.copperTrigger | 1 | Sales | 0 | 已加载 / 触发 |
| Cortex | n8n-nodes-base.cortex | 1 | Analytics, Development | 7 | 已加载 |
| CrateDB | n8n-nodes-base.crateDb | 1.1 | Data & Storage, Development | 3 | 已加载 |
| Cron | n8n-nodes-base.cron | 1 | Core Nodes | 0 | 已加载 / 触发 / 隐藏 |
| Crypto | n8n-nodes-base.crypto | 2 | Core Nodes, Development | 0 | 已加载 |
| Currents | n8n-nodes-base.currents | 1 |  | 30 | 已加载 |
| Currents Trigger | n8n-nodes-base.currentsTrigger | 1 |  | 0 | 已加载 / 触发 |
| Customer Datastore (n8n training) | n8n-nodes-base.n8nTrainingCustomerDatastore | 1 |  | 2 | 已加载 |
| Customer Messenger (n8n training) | n8n-nodes-base.n8nTrainingCustomerMessenger | 1 |  | 0 | 已加载 |
| Customer.io | n8n-nodes-base.customerIo | 1 | Communication, Development | 13 | 已加载 |
| Customer.io Trigger | n8n-nodes-base.customerIoTrigger | 1 | Communication, Development | 0 | 已加载 / 触发 |
| Data table | n8n-nodes-base.dataTable | 1.1 | Core Nodes, Development | 12 | 已加载 |
| Databricks | n8n-nodes-base.databricks | 1 | Data & Storage | 53 | 已加载 |
| Databricks Trigger | n8n-nodes-base.databricksTrigger | 1 | Data & Storage | 0 | 已加载 / 触发 |
| Date & Time | n8n-nodes-base.dateTime | 2 | Core Nodes | 7 | 已加载 |
| DebugHelper | n8n-nodes-base.debugHelper | 1 | Development | 0 | 已加载 |
| DeepL | n8n-nodes-base.deepL | 1 | Utility | 2 | 已加载 |
| Demio | n8n-nodes-base.demio | 1 | Communication | 4 | 已加载 |
| DHL | n8n-nodes-base.dhl | 1 | Miscellaneous | 1 | 已加载 |
| Discord | n8n-nodes-base.discord | 2 | Communication, HITL | 23 | 已加载 |
| Discourse | n8n-nodes-base.discourse | 1 | Communication | 21 | 已加载 |
| Disqus | n8n-nodes-base.disqus | 1 | Communication | 4 | 已加载 |
| Drift | n8n-nodes-base.drift | 1 | Sales | 6 | 已加载 |
| Dropbox | n8n-nodes-base.dropbox | 1 | Data & Storage | 14 | 已加载 |
| Dropcontact | n8n-nodes-base.dropcontact | 1 | Sales | 3 | 已加载 |
| E-goi | n8n-nodes-base.egoi | 1 | Communication, Marketing | 4 | 已加载 |
| E2E Test | n8n-nodes-base.e2eTest | 1 | Core Nodes | 3 | 未加载 |
| E2E Test Polling Trigger | n8n-nodes-base.e2eTestPollingTrigger | 1 | Core Nodes | 0 | 已加载 / 触发 |
| Edit Fields (Set) | n8n-nodes-base.set | 3.5 | Core Nodes | 0 | 已加载 |
| Edit Image | n8n-nodes-base.editImage | 1.1 | Core Nodes, Marketing | 24 | 已加载 |
| Elastic Security | n8n-nodes-base.elasticSecurity | 1 | Development | 18 | 已加载 |
| Elasticsearch | n8n-nodes-base.elasticsearch | 1 | Data & Storage, Development | 11 | 已加载 |
| Email Trigger (IMAP) | n8n-nodes-base.emailReadImap | 2.2 | Communication, Core Nodes | 0 | 已加载 / 触发 |
| Emelia | n8n-nodes-base.emelia | 1 | Communication, Marketing | 9 | 已加载 |
| Emelia Trigger | n8n-nodes-base.emeliaTrigger | 1 | Communication, Marketing | 0 | 已加载 / 触发 |
| ERPNext | n8n-nodes-base.erpNext | 1 | Data & Storage, Finance & Accounting | 6 | 已加载 |
| Error Trigger | n8n-nodes-base.errorTrigger | 1 | Core Nodes, Development | 0 | 已加载 / 触发 |
| Evaluation | n8n-nodes-base.evaluation | 4.8 | Utility | 5 | 已加载 |
| Evaluation Trigger | n8n-nodes-base.evaluationTrigger | 4.7 |  | 0 | 已加载 / 触发 |
| Eventbrite Trigger | n8n-nodes-base.eventbriteTrigger | 1 | Sales | 0 | 已加载 / 触发 |
| Execute Command | n8n-nodes-base.executeCommand | 1 | Core Nodes, Development | 0 | 未加载 |
| Execute Sub-workflow | n8n-nodes-base.executeWorkflow | 1.4 | Core Nodes | 1 | 已加载 |
| Execute Workflow Trigger | n8n-nodes-base.executeWorkflowTrigger | 1.3 | Core Nodes | 0 | 已加载 / 触发 |
| Execution Data | n8n-nodes-base.executionData | 1.1 | Core Nodes, Development | 1 | 已加载 |
| Extract from File | n8n-nodes-base.extractFromFile | 1.2 | Core Nodes | 12 | 已加载 |
| Facebook Graph API | n8n-nodes-base.facebookGraphApi | 1 | Development | 0 | 已加载 |
| Facebook Lead Ads Trigger | n8n-nodes-base.facebookLeadAdsTrigger | 1 | Marketing | 0 | 已加载 / 触发 |
| Facebook Trigger | n8n-nodes-base.facebookTrigger | 1 | Marketing | 0 | 已加载 / 触发 |
| Figma Trigger (Beta) | n8n-nodes-base.figmaTrigger | 1 | Miscellaneous | 0 | 已加载 / 触发 |
| FileMaker | n8n-nodes-base.filemaker | 1 | Data & Storage, Development | 0 | 已加载 |
| Filter | n8n-nodes-base.filter | 2.4 | Core Nodes | 0 | 已加载 |
| Flow | n8n-nodes-base.flow | 1 | Productivity | 4 | 已加载 |
| Flow Trigger | n8n-nodes-base.flowTrigger | 1 | Productivity | 0 | 已加载 / 触发 |
| Form.io Trigger | n8n-nodes-base.formIoTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| Formstack Trigger | n8n-nodes-base.formstackTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| Freshdesk | n8n-nodes-base.freshdesk | 1 | Communication | 12 | 已加载 |
| Freshservice | n8n-nodes-base.freshservice | 1 | Productivity | 72 | 已加载 |
| Freshworks CRM | n8n-nodes-base.freshworksCrm | 1 | Marketing, Sales | 40 | 已加载 |
| FTP | n8n-nodes-base.ftp | 1.1 | Core Nodes, Data & Storage, Development, Utility | 5 | 已加载 |
| Function | n8n-nodes-base.function | 1 | Core Nodes, Development | 0 | 已加载 / 隐藏 / 弃用 |
| Function Item | n8n-nodes-base.functionItem | 1 | Core Nodes, Development | 0 | 已加载 / 隐藏 / 弃用 |
| GetResponse | n8n-nodes-base.getResponse | 1 | Communication, Marketing | 6 | 已加载 |
| GetResponse Trigger | n8n-nodes-base.getResponseTrigger | 1 | Communication, Marketing | 0 | 已加载 / 触发 |
| Ghost | n8n-nodes-base.ghost | 1 | Marketing | 9 | 已加载 |
| Git | n8n-nodes-base.git | 1.1 | Core Nodes, Development | 15 | 已加载 |
| GitHub | n8n-nodes-base.github | 1.1 | Development | 57 | 已加载 |
| Github Trigger | n8n-nodes-base.githubTrigger | 1 | Development | 0 | 已加载 / 触发 |
| GitLab | n8n-nodes-base.gitlab | 1 | Development | 23 | 已加载 |
| GitLab Trigger | n8n-nodes-base.gitlabTrigger | 1 | Development | 0 | 已加载 / 触发 |
| Gmail | n8n-nodes-base.gmail | 2.2 | Communication, HITL | 26 | 已加载 |
| Gmail Trigger | n8n-nodes-base.gmailTrigger | 1.4 | Communication | 0 | 已加载 / 触发 |
| Gong | n8n-nodes-base.gong | 1 | Developer Tools, Development | 6 | 已加载 |
| Google Ads | n8n-nodes-base.googleAds | 1 | Analytics | 3 | 已加载 |
| Google Analytics | n8n-nodes-base.googleAnalytics | 2 | Analytics | 14 | 已加载 |
| Google BigQuery | n8n-nodes-base.googleBigQuery | 2.1 | Data & Storage, Development | 2 | 已加载 |
| Google Books | n8n-nodes-base.googleBooks | 2 | Miscellaneous | 12 | 已加载 |
| Google Business Profile | n8n-nodes-base.googleBusinessProfile | 1 | Marketing, Productivity | 11 | 已加载 |
| Google Business Profile Trigger | n8n-nodes-base.googleBusinessProfileTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| Google Calendar | n8n-nodes-base.googleCalendar | 1.3 | Productivity | 8 | 已加载 |
| Google Calendar Trigger | n8n-nodes-base.googleCalendarTrigger | 1 | Productivity | 0 | 已加载 / 触发 |
| Google Chat | n8n-nodes-base.googleChat | 1 | Communication, HITL | 12 | 已加载 |
| Google Cloud Firestore | n8n-nodes-base.googleFirebaseCloudFirestore | 1.1 | Data & Storage | 9 | 已加载 |
| Google Cloud Natural Language | n8n-nodes-base.googleCloudNaturalLanguage | 1 | Analytics | 2 | 已加载 |
| Google Cloud Realtime Database | n8n-nodes-base.googleFirebaseRealtimeDatabase | 1 | Data & Storage | 6 | 已加载 |
| Google Cloud Storage | n8n-nodes-base.googleCloudStorage | 1.1 | Data & Storage, Development | 10 | 已加载 |
| Google Contacts | n8n-nodes-base.googleContacts | 1 | Miscellaneous | 6 | 已加载 |
| Google Docs | n8n-nodes-base.googleDocs | 2 | Miscellaneous | 4 | 已加载 |
| Google Drive | n8n-nodes-base.googleDrive | 3 | Data & Storage | 21 | 已加载 |
| Google Drive Trigger | n8n-nodes-base.googleDriveTrigger | 1 | Data & Storage | 0 | 已加载 / 触发 |
| Google Perspective | n8n-nodes-base.googlePerspective | 1 | Analytics, Utility | 2 | 已加载 |
| Google Sheets | n8n-nodes-base.googleSheets | 4.7 | Data & Storage, Productivity | 10 | 已加载 |
| Google Sheets Trigger | n8n-nodes-base.googleSheetsTrigger | 1 | Data & Storage, Productivity | 0 | 已加载 / 触发 |
| Google Slides | n8n-nodes-base.googleSlides | 2 | Marketing | 8 | 已加载 |
| Google Tasks | n8n-nodes-base.googleTasks | 1 | Productivity | 6 | 已加载 |
| Google Translate | n8n-nodes-base.googleTranslate | 2 | Utility | 2 | 已加载 |
| Google Workspace Admin | n8n-nodes-base.gSuiteAdmin | 1 | Utility | 19 | 已加载 |
| Gotify | n8n-nodes-base.gotify | 1 | Communication | 3 | 已加载 |
| GoToWebinar | n8n-nodes-base.goToWebinar | 1 | Communication | 28 | 已加载 |
| Grafana | n8n-nodes-base.grafana | 1 | Analytics, Development | 20 | 已加载 |
| GraphQL | n8n-nodes-base.graphql | 1.1 | Data & Storage, Development | 0 | 已加载 |
| Grist | n8n-nodes-base.grist | 1 | Data & Storage | 6 | 已加载 |
| Gumroad Trigger | n8n-nodes-base.gumroadTrigger | 1 | Sales | 0 | 已加载 / 触发 |
| Hacker News | n8n-nodes-base.hackerNews | 1 | Communication, Marketing | 3 | 已加载 |
| HaloPSA | n8n-nodes-base.haloPSA | 1 | Communication | 20 | 已加载 |
| Harvest | n8n-nodes-base.harvest | 1 | Productivity | 61 | 已加载 |
| Help Scout | n8n-nodes-base.helpScout | 1 | Communication | 17 | 已加载 |
| Help Scout Trigger | n8n-nodes-base.helpScoutTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| HighLevel | n8n-nodes-base.highLevel | 2 | Marketing, Sales | 21 | 已加载 |
| Home Assistant | n8n-nodes-base.homeAssistant | 1 | Miscellaneous | 14 | 已加载 |
| HTML | n8n-nodes-base.html | 1.2 | Core Nodes | 3 | 已加载 |
| HTML Extract | n8n-nodes-base.htmlExtract | 1 | Core Nodes | 0 | 已加载 / 隐藏 |
| HTTP Request | n8n-nodes-base.httpRequest | 4.5 | Core Nodes, Development | 0 | 已加载 |
| HubSpot | n8n-nodes-base.hubspot | 2.2 | Sales | 31 | 已加载 |
| HubSpot Trigger | n8n-nodes-base.hubspotTrigger | 1 | Sales | 0 | 已加载 / 触发 |
| Humantic AI | n8n-nodes-base.humanticAi | 1 | Analytics | 3 | 已加载 |
| Hunter | n8n-nodes-base.hunter | 1 | Sales | 3 | 已加载 |
| iCalendar | n8n-nodes-base.iCal | 1 | Core Nodes, Productivity | 1 | 已加载 / 隐藏 |
| If | n8n-nodes-base.if | 2.4 | Core Nodes | 0 | 已加载 |
| Intercom | n8n-nodes-base.intercom | 1 | Communication, Sales | 18 | 已加载 |
| Interval | n8n-nodes-base.interval | 1 | Core Nodes | 0 | 已加载 / 触发 / 隐藏 |
| Invoice Ninja | n8n-nodes-base.invoiceNinja | 2 | Finance & Accounting | 38 | 已加载 |
| Invoice Ninja Trigger | n8n-nodes-base.invoiceNinjaTrigger | 2 | Finance & Accounting | 0 | 已加载 / 触发 |
| Item Lists | n8n-nodes-base.itemLists | 3.1 | Core Nodes | 6 | 已加载 / 隐藏 |
| Iterable | n8n-nodes-base.iterable | 1 | Communication, Marketing | 9 | 已加载 |
| Jenkins | n8n-nodes-base.jenkins | 1 | Development | 11 | 已加载 |
| Jina AI | n8n-nodes-base.jinaAi | 1 | Miscellaneous | 5 | 已加载 |
| Jira Software | n8n-nodes-base.jira | 1 | Development, Productivity | 24 | 已加载 |
| Jira Trigger | n8n-nodes-base.jiraTrigger | 1.1 | Development, Productivity | 0 | 已加载 / 触发 |
| Jotform Trigger | n8n-nodes-base.jotFormTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| JWT | n8n-nodes-base.jwt | 1 | Development | 3 | 已加载 |
| Kafka | n8n-nodes-base.kafka | 2 | Development | 0 | 已加载 |
| Kafka Trigger | n8n-nodes-base.kafkaTrigger | 2 | Development | 0 | 已加载 / 触发 |
| Keap | n8n-nodes-base.keap | 1 | Communication, Sales | 36 | 已加载 |
| Keap Trigger | n8n-nodes-base.keapTrigger | 1 | Communication, Sales | 0 | 已加载 / 触发 |
| KoBoToolbox | n8n-nodes-base.koBoToolbox | 1 | Communication, Data & Storage | 21 | 已加载 |
| KoBoToolbox Trigger | n8n-nodes-base.koBoToolboxTrigger | 1 | Communication, Data & Storage | 0 | 已加载 / 触发 |
| Ldap | n8n-nodes-base.ldap | 1 | Developer Tools, Development | 6 | 已加载 |
| Lemlist | n8n-nodes-base.lemlist | 2 | Communication, Marketing | 21 | 已加载 |
| Lemlist Trigger | n8n-nodes-base.lemlistTrigger | 1 | Communication, Marketing | 0 | 已加载 / 触发 |
| Limit | n8n-nodes-base.limit | 1 | Core Nodes | 0 | 已加载 |
| Line | n8n-nodes-base.line | 1 | Communication | 2 | 已加载 / 隐藏 |
| Linear | n8n-nodes-base.linear | 1.1 | Productivity | 9 | 已加载 |
| Linear Trigger | n8n-nodes-base.linearTrigger | 1 | Productivity | 0 | 已加载 / 触发 |
| LingvaNex | n8n-nodes-base.lingvaNex | 1 | Miscellaneous | 1 | 已加载 |
| LinkedIn | n8n-nodes-base.linkedIn | 1 | Communication, Marketing | 2 | 已加载 |
| Local File Trigger | n8n-nodes-base.localFileTrigger | 1 | Core Nodes | 0 | 未加载 / 触发 |
| LoneScale | n8n-nodes-base.loneScale | 1 | Sales | 9 | 已加载 |
| LoneScale Trigger | n8n-nodes-base.loneScaleTrigger | 1 | Sales | 0 | 已加载 / 触发 |
| Loop Over Items (Split in Batches) | n8n-nodes-base.splitInBatches | 3 | Core Nodes | 0 | 已加载 |
| Magento 2 | n8n-nodes-base.magento2 | 1 | Sales | 19 | 已加载 |
| Mailcheck | n8n-nodes-base.mailcheck | 1 | Marketing, Utility | 1 | 已加载 |
| Mailchimp | n8n-nodes-base.mailchimp | 1 | Communication, Marketing | 18 | 已加载 |
| Mailchimp Trigger | n8n-nodes-base.mailchimpTrigger | 1 | Communication, Marketing | 0 | 已加载 / 触发 |
| MailerLite | n8n-nodes-base.mailerLite | 2 | Communication, Marketing | 4 | 已加载 |
| MailerLite Trigger | n8n-nodes-base.mailerLiteTrigger | 2 | Communication, Marketing | 0 | 已加载 / 触发 |
| Mailgun | n8n-nodes-base.mailgun | 1 | Communication, Development | 0 | 已加载 |
| Mailjet | n8n-nodes-base.mailjet | 1 | Communication, Development | 5 | 已加载 |
| Mailjet Trigger | n8n-nodes-base.mailjetTrigger | 1 | Communication, Development | 0 | 已加载 / 触发 |
| Mandrill | n8n-nodes-base.mandrill | 1 | Communication, Development | 2 | 已加载 |
| Manual Trigger | n8n-nodes-base.manualTrigger | 1 | Core Nodes | 0 | 已加载 / 触发 |
| Markdown | n8n-nodes-base.markdown | 1 | Core Nodes | 0 | 已加载 |
| Marketstack | n8n-nodes-base.marketstack | 1 | Analytics, Finance & Accounting | 3 | 已加载 |
| Matrix | n8n-nodes-base.matrix | 1 | Communication | 11 | 已加载 |
| Mattermost | n8n-nodes-base.mattermost | 1 | Communication | 23 | 已加载 |
| Mautic | n8n-nodes-base.mautic | 1 | Communication, Marketing | 26 | 已加载 |
| Mautic Trigger | n8n-nodes-base.mauticTrigger | 1 | Communication, Marketing | 0 | 已加载 / 触发 |
| Medium | n8n-nodes-base.medium | 1 | Marketing | 4 | 已加载 |
| Merge | n8n-nodes-base.merge | 3.2 | Core Nodes | 0 | 已加载 |
| Message an Agent | n8n-nodes-base.messageAnAgent | 3.1 | AI | 0 | 已加载 |
| MessageBird | n8n-nodes-base.messageBird | 1 | Communication | 2 | 已加载 |
| Metabase | n8n-nodes-base.metabase | 1 | Data & Storage, Development | 14 | 已加载 |
| Microsoft Dataverse | n8n-nodes-base.microsoftDataverse | 1 | Data & Storage, Development, Finance & Accounting, Sales | 7 | 已加载 |
| Microsoft Dynamics CRM | n8n-nodes-base.microsoftDynamicsCrm | 1 | Marketing, Sales | 6 | 已加载 |
| Microsoft Entra ID | n8n-nodes-base.microsoftEntra | 1 | Developer Tools, Development | 14 | 已加载 |
| Microsoft Excel (OneDrive) | n8n-nodes-base.microsoftExcel | 2.2 | Data & Storage, Productivity | 17 | 已加载 |
| Microsoft Excel (SharePoint) | n8n-nodes-base.microsoftExcelSharePoint | 1 | Data & Storage, Productivity | 21 | 已加载 |
| Microsoft Graph Security | n8n-nodes-base.microsoftGraphSecurity | 1 | Development | 7 | 已加载 |
| Microsoft OneDrive | n8n-nodes-base.microsoftOneDrive | 1.1 | Data & Storage | 18 | 已加载 |
| Microsoft OneDrive Trigger | n8n-nodes-base.microsoftOneDriveTrigger | 1 | Data & Storage | 0 | 已加载 / 触发 |
| Microsoft Outlook | n8n-nodes-base.microsoftOutlook | 2 | Communication, HITL | 46 | 已加载 |
| Microsoft Outlook Trigger | n8n-nodes-base.microsoftOutlookTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| Microsoft SharePoint | n8n-nodes-base.microsoftSharePoint | 2 |  | 14 | 已加载 |
| Microsoft SharePoint Trigger | n8n-nodes-base.microsoftSharePointTrigger | 1 | ECM | 0 | 已加载 / 触发 |
| Microsoft SQL | n8n-nodes-base.microsoftSql | 1.2 | Data & Storage, Development | 4 | 已加载 |
| Microsoft Teams | n8n-nodes-base.microsoftTeams | 2 | Communication, HITL | 43 | 已加载 |
| Microsoft Teams Trigger | n8n-nodes-base.microsoftTeamsTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| Microsoft To Do | n8n-nodes-base.microsoftToDo | 1 | Productivity | 18 | 已加载 |
| Mindee | n8n-nodes-base.mindee | 3 | Utility | 2 | 已加载 |
| MISP | n8n-nodes-base.misp | 1 | Development | 55 | 已加载 |
| Mistral AI | n8n-nodes-base.mistralAi | 1 | Utility | 2 | 已加载 |
| Mocean | n8n-nodes-base.mocean | 1 | Communication | 1 | 已加载 |
| Monday.com | n8n-nodes-base.mondayCom | 2 | Productivity | 44 | 已加载 |
| MongoDB | n8n-nodes-base.mongoDb | 1.5 | Data & Storage, Development | 11 | 已加载 |
| Monica CRM | n8n-nodes-base.monicaCrm | 1 | Communication | 52 | 已加载 |
| MQTT | n8n-nodes-base.mqtt | 1 | Communication, Development | 0 | 已加载 |
| MQTT Trigger | n8n-nodes-base.mqttTrigger | 1 | Communication, Development | 0 | 已加载 / 触发 |
| MSG91 | n8n-nodes-base.msg91 | 1 | Communication, Development | 1 | 已加载 |
| MySQL | n8n-nodes-base.mySql | 2.5 | Data & Storage, Development | 6 | 已加载 |
| n8n | n8n-nodes-base.n8n | 1 | Core Nodes, Development | 19 | 已加载 |
| n8n Form | n8n-nodes-base.form | 2.5 | Core Nodes | 2 | 已加载 |
| n8n Form Trigger | n8n-nodes-base.formTrigger | 2.6 | Core Nodes | 0 | 已加载 / 触发 |
| n8n Trigger | n8n-nodes-base.n8nTrigger | 1 | Core Nodes | 0 | 已加载 / 触发 |
| NASA | n8n-nodes-base.nasa | 1 | Miscellaneous | 40 | 已加载 |
| Netlify | n8n-nodes-base.netlify | 1 | Development | 9 | 已加载 |
| Netlify Trigger | n8n-nodes-base.netlifyTrigger | 1 | Development | 0 | 已加载 / 触发 |
| Netscaler ADC | n8n-nodes-base.citrixAdc | 1 | Development | 7 | 已加载 |
| Nextcloud | n8n-nodes-base.nextCloud | 1 | Data & Storage | 20 | 已加载 |
| No Operation, do nothing | n8n-nodes-base.noOp | 1 | Core Nodes | 0 | 已加载 |
| NocoDB | n8n-nodes-base.nocoDb | 4 | Data & Storage | 13 | 已加载 |
| Notion | n8n-nodes-base.notion | 3 | Productivity | 23 | 已加载 |
| Notion Trigger | n8n-nodes-base.notionTrigger | 1.1 | Productivity | 0 | 已加载 / 触发 |
| Npm | n8n-nodes-base.npm | 1 | Development | 7 | 已加载 |
| Odoo | n8n-nodes-base.odoo | 2 | Data & Storage | 24 | 已加载 |
| Okta | n8n-nodes-base.okta | 1 | Development | 6 | 已加载 |
| One Simple API | n8n-nodes-base.oneSimpleApi | 1 | Utility | 10 | 已加载 |
| Onfleet | n8n-nodes-base.onfleet | 1 | Miscellaneous | 37 | 已加载 |
| Onfleet Trigger | n8n-nodes-base.onfleetTrigger | 1 | Miscellaneous | 0 | 已加载 / 触发 |
| OpenAI | n8n-nodes-base.openAi | 1.1 | Utility | 8 | 已加载 / 隐藏 |
| OpenThesaurus | n8n-nodes-base.openThesaurus | 1 | Utility | 1 | 已加载 |
| OpenWeatherMap | n8n-nodes-base.openWeatherMap | 1 | Miscellaneous, Utility | 3 | 已加载 |
| Oracle Database | n8n-nodes-base.oracleDatabase | 1.1 |  | 6 | 已加载 |
| Orbit | n8n-nodes-base.orbit | 1 | Analytics | 14 | 已加载 / 隐藏 |
| Oura | n8n-nodes-base.oura | 1 | Productivity | 6 | 已加载 |
| Paddle | n8n-nodes-base.paddle | 1 | Sales | 9 | 已加载 |
| PagerDuty | n8n-nodes-base.pagerDuty | 1 | Communication, Development | 13 | 已加载 |
| PayPal | n8n-nodes-base.payPal | 1 | Finance & Accounting, Sales | 4 | 已加载 |
| PayPal Trigger | n8n-nodes-base.payPalTrigger | 1 | Finance & Accounting, Sales | 0 | 已加载 / 触发 |
| Peekalink | n8n-nodes-base.peekalink | 1 | Development | 3 | 已加载 |
| Perplexity | n8n-nodes-base.perplexity | 3 | AI, Utility | 4 | 已加载 |
| Phantombuster | n8n-nodes-base.phantombuster | 1 | Marketing, Sales | 7 | 已加载 |
| Philips Hue | n8n-nodes-base.philipsHue | 1 | Miscellaneous | 5 | 已加载 |
| Pipedrive | n8n-nodes-base.pipedrive | 2 | Sales | 58 | 已加载 |
| Pipedrive Trigger | n8n-nodes-base.pipedriveTrigger | 1.1 | Sales | 0 | 已加载 / 触发 |
| Plivo | n8n-nodes-base.plivo | 1 | Communication, Development | 3 | 已加载 |
| PostBin | n8n-nodes-base.postBin | 1 | Data & Storage, Development | 6 | 已加载 |
| Postgres | n8n-nodes-base.postgres | 2.8 | Data & Storage, Development | 6 | 已加载 |
| Postgres Trigger | n8n-nodes-base.postgresTrigger | 1 | Development | 0 | 已加载 / 触发 |
| PostHog | n8n-nodes-base.postHog | 1 | Analytics, Development | 9 | 已加载 |
| Postmark Trigger | n8n-nodes-base.postmarkTrigger | 1 | Communication, Development | 0 | 已加载 / 触发 |
| ProfitWell | n8n-nodes-base.profitWell | 1 | Analytics | 2 | 已加载 |
| Pushbullet | n8n-nodes-base.pushbullet | 1 | Communication | 5 | 已加载 |
| Pushcut | n8n-nodes-base.pushcut | 1 | Communication, Productivity | 1 | 已加载 |
| Pushcut Trigger | n8n-nodes-base.pushcutTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| Pushover | n8n-nodes-base.pushover | 1 | Communication | 2 | 已加载 |
| QuestDB | n8n-nodes-base.questDb | 1.1 | Data & Storage, Development | 2 | 已加载 |
| Quick Base | n8n-nodes-base.quickbase | 1 | Data & Storage | 10 | 已加载 |
| QuickBooks Online | n8n-nodes-base.quickbooks | 1 | Finance & Accounting | 52 | 已加载 |
| QuickChart | n8n-nodes-base.quickChart | 1 | Marketing | 0 | 已加载 |
| RabbitMQ | n8n-nodes-base.rabbitmq | 1.2 | Development | 4 | 已加载 |
| RabbitMQ Trigger | n8n-nodes-base.rabbitmqTrigger | 1 | Development | 0 | 已加载 / 触发 |
| Raindrop | n8n-nodes-base.raindrop | 1 | Productivity | 17 | 已加载 |
| Read Binary File | n8n-nodes-base.readBinaryFile | 1 | Core Nodes | 0 | 已加载 / 隐藏 |
| Read Binary Files | n8n-nodes-base.readBinaryFiles | 1 | Core Nodes | 0 | 已加载 / 隐藏 |
| Read PDF | n8n-nodes-base.readPDF | 1 | Core Nodes | 0 | 已加载 / 隐藏 |
| Read/Write Files from Disk | n8n-nodes-base.readWriteFile | 1.1 | Core Nodes | 2 | 已加载 |
| Reddit | n8n-nodes-base.reddit | 1 | Communication | 18 | 已加载 |
| Redis | n8n-nodes-base.redis | 1 | Data & Storage, Development | 10 | 已加载 |
| Redis Trigger | n8n-nodes-base.redisTrigger | 1 | Communication, Data & Storage, Development | 0 | 已加载 / 触发 |
| Remove Duplicates | n8n-nodes-base.removeDuplicates | 2 | Core Nodes | 3 | 已加载 |
| Rename Keys | n8n-nodes-base.renameKeys | 1 | Core Nodes | 0 | 已加载 |
| Respond to Webhook | n8n-nodes-base.respondToWebhook | 1.5 | Core Nodes, Utility | 0 | 已加载 |
| RocketChat | n8n-nodes-base.rocketchat | 1 | Communication | 7 | 已加载 |
| RSS Feed Trigger | n8n-nodes-base.rssFeedReadTrigger | 1 | Core Nodes | 0 | 已加载 / 触发 |
| RSS Read | n8n-nodes-base.rssFeedRead | 1.2 | Core Nodes | 0 | 已加载 |
| Rundeck | n8n-nodes-base.rundeck | 1.1 | Communication, Development | 3 | 已加载 |
| S3 | n8n-nodes-base.s3 | 1 | Data & Storage, Development | 12 | 已加载 |
| Salesforce | n8n-nodes-base.salesforce | 1.1 | Communication, Sales | 82 | 已加载 |
| Salesforce Trigger | n8n-nodes-base.salesforceTrigger | 1.1 | Communication, Sales | 0 | 已加载 / 触发 |
| Salesmate | n8n-nodes-base.salesmate | 1 | Sales | 15 | 已加载 |
| Schedule Trigger | n8n-nodes-base.scheduleTrigger | 1.4 | Core Nodes | 0 | 已加载 / 触发 |
| SeaTable | n8n-nodes-base.seaTable | 2 | Data & Storage | 16 | 已加载 |
| SeaTable Trigger | n8n-nodes-base.seaTableTrigger | 2 | Data & Storage | 0 | 已加载 / 触发 |
| SecurityScorecard | n8n-nodes-base.securityScorecard | 1 | Analytics | 19 | 已加载 |
| Segment | n8n-nodes-base.segment | 1 | Analytics, Development | 7 | 已加载 |
| Send Email | n8n-nodes-base.emailSend | 2.1 | Communication, Core Nodes, HITL | 2 | 已加载 |
| SendGrid | n8n-nodes-base.sendGrid | 1 | Communication, Marketing | 13 | 已加载 |
| Sendy | n8n-nodes-base.sendy | 1 | Communication, Marketing | 6 | 已加载 |
| Sentry.io | n8n-nodes-base.sentryIo | 1 | Development | 31 | 已加载 |
| ServiceNow | n8n-nodes-base.serviceNow | 1 | Communication, Productivity | 35 | 已加载 |
| seven | n8n-nodes-base.sms77 | 1 | Communication | 4 | 已加载 |
| Shopify | n8n-nodes-base.shopify | 1 | Sales | 12 | 已加载 |
| Shopify Trigger | n8n-nodes-base.shopifyTrigger | 1 | Sales | 0 | 已加载 / 触发 |
| SIGNL4 | n8n-nodes-base.signl4 | 1 | Communication, Development | 2 | 已加载 |
| Simulate | n8n-nodes-base.simulate | 1 | Core Nodes | 0 | 已加载 / 隐藏 |
| Simulate Trigger | n8n-nodes-base.simulateTrigger | 1 | Core Nodes | 0 | 已加载 / 触发 / 隐藏 |
| Slack | n8n-nodes-base.slack | 2.7 | Communication, HITL | 48 | 已加载 |
| Slack Trigger | n8n-nodes-base.slackTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| Snowflake | n8n-nodes-base.snowflake | 1.1 | Data & Storage | 4 | 已加载 |
| Sort | n8n-nodes-base.sort | 1 | Core Nodes | 0 | 已加载 |
| Split Out | n8n-nodes-base.splitOut | 1.1 | Core Nodes | 0 | 已加载 |
| Splunk | n8n-nodes-base.splunk | 2 | Development | 20 | 已加载 |
| Spotify | n8n-nodes-base.spotify | 1 | Miscellaneous | 37 | 已加载 |
| Spreadsheet File | n8n-nodes-base.spreadsheetFile | 2 | Core Nodes, Data & Storage | 2 | 已加载 / 隐藏 |
| SSE Trigger | n8n-nodes-base.sseTrigger | 1 | Core Nodes, Development | 0 | 已加载 / 触发 |
| SSH | n8n-nodes-base.ssh | 1 | Core Nodes, Development | 3 | 已加载 |
| Stackby | n8n-nodes-base.stackby | 1 | Data & Storage | 4 | 已加载 |
| Sticky Note | n8n-nodes-base.stickyNote | 1 |  | 0 | 已加载 |
| Stop and Error | n8n-nodes-base.stopAndError | 1 | Core Nodes, Utility | 0 | 已加载 |
| Storyblok | n8n-nodes-base.storyblok | 1 | Data & Storage, Marketing | 7 | 已加载 |
| Strapi | n8n-nodes-base.strapi | 1 | Data & Storage, Marketing | 6 | 已加载 |
| Strava | n8n-nodes-base.strava | 1.1 | Productivity | 10 | 已加载 |
| Strava Trigger | n8n-nodes-base.stravaTrigger | 1 | Productivity | 0 | 已加载 / 触发 |
| Stripe | n8n-nodes-base.stripe | 1 | Finance & Accounting, Sales | 28 | 已加载 |
| Stripe Trigger | n8n-nodes-base.stripeTrigger | 1 | Finance & Accounting, Sales | 0 | 已加载 / 触发 |
| Summarize | n8n-nodes-base.summarize | 1.1 | Core Nodes | 0 | 已加载 |
| Supabase | n8n-nodes-base.supabase | 1 | Data & Storage | 6 | 已加载 |
| SurveyMonkey Trigger | n8n-nodes-base.surveyMonkeyTrigger | 1 | Communication, Marketing | 0 | 已加载 / 触发 |
| Switch | n8n-nodes-base.switch | 3.5 | Core Nodes | 0 | 已加载 |
| SyncroMSP | n8n-nodes-base.syncroMsp | 1 | Productivity | 20 | 已加载 |
| Taiga | n8n-nodes-base.taiga | 1 | Development, Productivity | 20 | 已加载 |
| Taiga Trigger | n8n-nodes-base.taigaTrigger | 1 | Development, Productivity | 0 | 已加载 / 触发 |
| Tapfiliate | n8n-nodes-base.tapfiliate | 1 | Sales | 12 | 已加载 |
| Telegram | n8n-nodes-base.telegram | 1.2 | Communication, HITL | 27 | 已加载 |
| Telegram Trigger | n8n-nodes-base.telegramTrigger | 1.5 | Communication | 0 | 已加载 / 触发 |
| TheHive | n8n-nodes-base.theHive | 1 | Development | 5 | 已加载 |
| TheHive 5 | n8n-nodes-base.theHiveProject | 1 | Development | 56 | 已加载 |
| TheHive 5 Trigger | n8n-nodes-base.theHiveProjectTrigger | 1 | Development | 0 | 已加载 / 触发 |
| TheHive Trigger | n8n-nodes-base.theHiveTrigger | 2 | Development | 0 | 已加载 / 触发 |
| TimescaleDB | n8n-nodes-base.timescaleDb | 1.1 | Data & Storage, Development | 3 | 已加载 |
| Todoist | n8n-nodes-base.todoist | 2.2 | Productivity | 36 | 已加载 |
| Toggl Trigger | n8n-nodes-base.togglTrigger | 1 | Productivity, Utility | 0 | 已加载 / 触发 |
| TOTP | n8n-nodes-base.totp | 1 | Core Nodes | 1 | 已加载 |
| Track Time Saved | n8n-nodes-base.timeSaved | 1 | Core Nodes, Development | 0 | 已加载 |
| TravisCI | n8n-nodes-base.travisCi | 1 | Development | 5 | 已加载 |
| Trello | n8n-nodes-base.trello | 1 | Productivity | 49 | 已加载 |
| Trello Trigger | n8n-nodes-base.trelloTrigger | 1 | Productivity | 0 | 已加载 / 触发 |
| Twake | n8n-nodes-base.twake | 1 | Productivity | 2 | 已加载 |
| Twilio | n8n-nodes-base.twilio | 1 | Communication, Development | 4 | 已加载 |
| Twilio Trigger | n8n-nodes-base.twilioTrigger | 1 | Communication, Development | 0 | 已加载 / 触发 |
| Twist | n8n-nodes-base.twist | 1 | Communication | 26 | 已加载 |
| Typeform Trigger | n8n-nodes-base.typeformTrigger | 1.1 | Communication | 0 | 已加载 / 触发 |
| Unleashed Software | n8n-nodes-base.unleashedSoftware | 1.1 | Data & Storage, Sales | 3 | 已加载 |
| Uplead | n8n-nodes-base.uplead | 1 | Sales | 2 | 已加载 |
| uProc | n8n-nodes-base.uproc | 1 | Data & Storage | 0 | 已加载 |
| UptimeRobot | n8n-nodes-base.uptimeRobot | 1 | Development | 26 | 已加载 |
| urlscan.io | n8n-nodes-base.urlScanIo | 1 | Development, Utility | 4 | 已加载 |
| Venafi TLS Protect Cloud | n8n-nodes-base.venafiTlsProtectCloud | 1 | Development | 10 | 已加载 |
| Venafi TLS Protect Cloud Trigger | n8n-nodes-base.venafiTlsProtectCloudTrigger | 1 | Development | 0 | 已加载 / 触发 |
| Venafi TLS Protect Datacenter | n8n-nodes-base.venafiTlsProtectDatacenter | 1 | Development | 9 | 已加载 |
| Vero | n8n-nodes-base.vero | 1 | Communication | 8 | 已加载 |
| Vonage | n8n-nodes-base.vonage | 1 | Communication | 1 | 已加载 |
| Wait | n8n-nodes-base.wait | 1.1 | Core Nodes | 0 | 已加载 |
| Webex by Cisco | n8n-nodes-base.ciscoWebex | 1 | Communication | 12 | 已加载 |
| Webex by Cisco Trigger | n8n-nodes-base.ciscoWebexTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| Webflow | n8n-nodes-base.webflow | 2 | Marketing | 5 | 已加载 |
| Webflow Trigger | n8n-nodes-base.webflowTrigger | 2 | Marketing | 0 | 已加载 / 触发 |
| Webhook | n8n-nodes-base.webhook | 2.2 | Core Nodes, Development | 0 | 已加载 / 触发 |
| Wekan | n8n-nodes-base.wekan | 1 | Productivity | 30 | 已加载 |
| WhatsApp Business Cloud | n8n-nodes-base.whatsApp | 1.1 | Communication, HITL | 6 | 已加载 |
| WhatsApp Trigger | n8n-nodes-base.whatsAppTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| Wise | n8n-nodes-base.wise | 1 | Finance & Accounting | 14 | 已加载 |
| Wise Trigger | n8n-nodes-base.wiseTrigger | 1 | Finance & Accounting | 0 | 已加载 / 触发 |
| WooCommerce | n8n-nodes-base.wooCommerce | 1 | Sales | 18 | 已加载 |
| WooCommerce Trigger | n8n-nodes-base.wooCommerceTrigger | 1 | Sales | 0 | 已加载 / 触发 |
| Wordpress | n8n-nodes-base.wordpress | 1 | Marketing | 15 | 已加载 |
| Workable Trigger | n8n-nodes-base.workableTrigger | 1 | Miscellaneous | 0 | 已加载 / 触发 |
| Workflow Trigger | n8n-nodes-base.workflowTrigger | 1 | Core Nodes | 0 | 已加载 / 触发 / 隐藏 |
| Write Binary File | n8n-nodes-base.writeBinaryFile | 1 | Core Nodes | 0 | 已加载 / 隐藏 |
| Wufoo Trigger | n8n-nodes-base.wufooTrigger | 1 | Communication | 0 | 已加载 / 触发 |
| X (Formerly Twitter) | n8n-nodes-base.twitter | 2 | Marketing | 12 | 已加载 |
| Xero | n8n-nodes-base.xero | 1 | Finance & Accounting | 10 | 已加载 |
| XML | n8n-nodes-base.xml | 1 | Core Nodes | 0 | 已加载 |
| Yourls | n8n-nodes-base.yourls | 1 | Utility | 3 | 已加载 |
| YouTube | n8n-nodes-base.youTube | 1 | Marketing | 25 | 已加载 |
| Zammad | n8n-nodes-base.zammad | 1 | Communication | 21 | 已加载 |
| Zendesk | n8n-nodes-base.zendesk | 1 | Communication | 27 | 已加载 |
| Zendesk Trigger | n8n-nodes-base.zendeskTrigger | 1 | Communication | 36 | 已加载 / 触发 |
| Zoho CRM | n8n-nodes-base.zohoCrm | 1 | Communication, Sales | 71 | 已加载 |
| Zoom | n8n-nodes-base.zoom | 1 | Communication | 6 | 已加载 |
| Zulip | n8n-nodes-base.zulip | 1 | Communication | 16 | 已加载 |

## AI 包完整节点表（127）

| 名称 | 类型标识 | 版本 | 分类 | 静态操作数 | 状态 |
|---|---|---|---|---:|---|
| AI Agent | @n8n/n8n-nodes-langchain.agent | 3.1 | AI | 0 | 已加载 |
| AI Agent Tool | @n8n/n8n-nodes-langchain.agentTool | 3 | AI | 0 | 已加载 |
| Anthropic | @n8n/n8n-nodes-langchain.anthropic | 1 | AI | 15 | 已加载 |
| Anthropic Chat Model | @n8n/n8n-nodes-langchain.lmChatAnthropic | 1.6 | AI | 0 | 已加载 |
| Auto-fixing Output Parser | @n8n/n8n-nodes-langchain.outputParserAutofixing | 1 | AI | 0 | 已加载 |
| AWS Bedrock Chat Model | @n8n/n8n-nodes-langchain.lmChatAwsBedrock | 1.2 | AI | 0 | 已加载 |
| Azure AI Search Vector Store | @n8n/n8n-nodes-langchain.vectorStoreAzureAISearch | 1.3 | AI | 0 | 已加载 |
| Basic LLM Chain | @n8n/n8n-nodes-langchain.chainLlm | 1.9 | AI | 0 | 已加载 |
| Binary Input Loader | @n8n/n8n-nodes-langchain.documentBinaryInputLoader | 1 | AI | 0 | 已加载 / 隐藏 |
| Calculator | @n8n/n8n-nodes-langchain.toolCalculator | 1 | AI | 0 | 已加载 |
| Call n8n Workflow Tool | @n8n/n8n-nodes-langchain.toolWorkflow | 2.2 | AI | 0 | 已加载 |
| Character Text Splitter | @n8n/n8n-nodes-langchain.textSplitterCharacterTextSplitter | 1 | AI | 0 | 已加载 |
| Chat | @n8n/n8n-nodes-langchain.chat | 1.3 | Core Nodes, HITL | 2 | 已加载 |
| Chat Memory Manager | @n8n/n8n-nodes-langchain.memoryManager | 1.1 | AI | 0 | 已加载 |
| Chat Messages Retriever | @n8n/n8n-nodes-langchain.memoryChatRetriever | 1 | AI | 0 | 已加载 / 隐藏 |
| Chat Trigger | @n8n/n8n-nodes-langchain.chatTrigger | 1.5 | Core Nodes | 0 | 已加载 / 触发 |
| ChatHub PGVector Store | @n8n/n8n-nodes-langchain.chatHubVectorStorePGVector | 1.3 | AI | 0 | 已加载 / 隐藏 |
| ChatHub Pinecone Vector Store | @n8n/n8n-nodes-langchain.chatHubVectorStorePinecone | 1.3 | AI | 0 | 已加载 / 隐藏 |
| ChatHub Qdrant Vector Store | @n8n/n8n-nodes-langchain.chatHubVectorStoreQdrant | 1.3 | AI | 0 | 已加载 / 隐藏 |
| Chroma Vector Store | @n8n/n8n-nodes-langchain.vectorStoreChromaDB | 1.3 | AI | 0 | 已加载 |
| Code Tool | @n8n/n8n-nodes-langchain.toolCode | 1.3 | AI | 0 | 已加载 |
| Cohere Chat Model | @n8n/n8n-nodes-langchain.lmChatCohere | 1 | AI | 0 | 已加载 |
| Cohere Model | @n8n/n8n-nodes-langchain.lmCohere | 1 | AI | 0 | 已加载 |
| Contextual Compression Retriever | @n8n/n8n-nodes-langchain.retrieverContextualCompression | 1 | AI | 0 | 已加载 |
| Databricks AI Agent | @n8n/n8n-nodes-langchain.databricksAiAgent | 2.2 | AI | 0 | 已加载 |
| Databricks Chat Model | @n8n/n8n-nodes-langchain.lmChatDatabricks | 1 | AI | 0 | 已加载 |
| Databricks Lakebase Chat Memory | @n8n/n8n-nodes-langchain.memoryLakebaseChat | 1 | AI | 0 | 已加载 / 隐藏 |
| Databricks Vector Store | @n8n/n8n-nodes-langchain.vectorStoreDatabricks | 1.3 | AI | 0 | 已加载 |
| DeepSeek Chat Model | @n8n/n8n-nodes-langchain.lmChatDeepSeek | 1 | AI | 0 | 已加载 |
| Default Data Loader | @n8n/n8n-nodes-langchain.documentDefaultDataLoader | 1.1 | AI | 0 | 已加载 |
| Embeddings AWS Bedrock | @n8n/n8n-nodes-langchain.embeddingsAwsBedrock | 1 | AI | 0 | 已加载 |
| Embeddings Cohere | @n8n/n8n-nodes-langchain.embeddingsCohere | 1 | AI | 0 | 已加载 |
| Embeddings Databricks | @n8n/n8n-nodes-langchain.embeddingsDatabricks | 1 | AI | 0 | 已加载 |
| Embeddings Google Gemini | @n8n/n8n-nodes-langchain.embeddingsGoogleGemini | 1 | AI | 0 | 已加载 |
| Embeddings Google Vertex | @n8n/n8n-nodes-langchain.embeddingsGoogleVertex | 1 | AI | 0 | 已加载 |
| Embeddings Hugging Face Inference | @n8n/n8n-nodes-langchain.embeddingsHuggingFaceInference | 1 | AI | 0 | 已加载 |
| Embeddings Lemonade | @n8n/n8n-nodes-langchain.embeddingsLemonade | 1 | AI | 0 | 已加载 |
| Embeddings Mistral Cloud | @n8n/n8n-nodes-langchain.embeddingsMistralCloud | 1 | AI | 0 | 已加载 |
| Embeddings Ollama | @n8n/n8n-nodes-langchain.embeddingsOllama | 1 | AI | 0 | 已加载 |
| Embeddings OpenAI | @n8n/n8n-nodes-langchain.embeddingsOpenAi | 1.2 | AI | 0 | 已加载 |
| Embeddings Oracle Database | @n8n/n8n-nodes-langchain.embeddingsOracleDb | 1 | AI | 0 | 已加载 |
| GitHub Document Loader | @n8n/n8n-nodes-langchain.documentGithubLoader | 1.1 | AI | 0 | 已加载 / 隐藏 |
| Google Gemini | @n8n/n8n-nodes-langchain.googleGemini | 1.2 | AI | 15 | 已加载 |
| Google Gemini Chat Model | @n8n/n8n-nodes-langchain.lmChatGoogleGemini | 1.2 | AI | 0 | 已加载 |
| Google Vertex Chat Model | @n8n/n8n-nodes-langchain.lmChatGoogleVertex | 1 | AI | 0 | 已加载 |
| Groq Chat Model | @n8n/n8n-nodes-langchain.lmChatGroq | 1 | AI | 0 | 已加载 |
| Guardrails | @n8n/n8n-nodes-langchain.guardrails | 2 | AI | 2 | 已加载 |
| HTTP Request Tool | @n8n/n8n-nodes-langchain.toolHttpRequest | 1.1 | AI | 0 | 已加载 / 隐藏 |
| Hugging Face Inference Model | @n8n/n8n-nodes-langchain.lmOpenHuggingFaceInference | 1 | AI | 0 | 已加载 |
| In Memory Vector Store Insert | @n8n/n8n-nodes-langchain.vectorStoreInMemoryInsert | 1 | AI | 0 | 已加载 / 隐藏 |
| In Memory Vector Store Load | @n8n/n8n-nodes-langchain.vectorStoreInMemoryLoad | 1 | AI | 0 | 已加载 / 隐藏 |
| Information Extractor | @n8n/n8n-nodes-langchain.informationExtractor | 1.2 | AI | 0 | 已加载 |
| Item List Output Parser | @n8n/n8n-nodes-langchain.outputParserItemList | 1 | AI | 0 | 已加载 |
| JSON Input Loader | @n8n/n8n-nodes-langchain.documentJsonInputLoader | 1 | AI | 0 | 已加载 / 隐藏 |
| LangChain Code | @n8n/n8n-nodes-langchain.code | 1 | AI | 0 | 已加载 / 隐藏 / 弃用 |
| Lemonade Chat Model | @n8n/n8n-nodes-langchain.lmChatLemonade | 1 | AI | 0 | 已加载 |
| Lemonade Model | @n8n/n8n-nodes-langchain.lmLemonade | 1 | AI | 0 | 已加载 |
| Manual Chat Trigger | @n8n/n8n-nodes-langchain.manualChatTrigger | 1.1 | Core Nodes | 0 | 已加载 / 触发 / 隐藏 |
| MCP Client | @n8n/n8n-nodes-langchain.mcpClient | 1.1 |  | 0 | 已加载 |
| MCP Client Tool | @n8n/n8n-nodes-langchain.mcpClientTool | 1.4 | AI | 0 | 已加载 |
| MCP Registry Client (internal) | @n8n/n8n-nodes-langchain.mcpRegistryClientTool | 1.1 | AI | 0 | 已加载 / 隐藏 |
| MCP Server Trigger | @n8n/n8n-nodes-langchain.mcpTrigger | 2.1 | AI, Core Nodes | 0 | 已加载 / 触发 |
| Microsoft Agent 365 Trigger | @n8n/n8n-nodes-langchain.microsoftAgent365Trigger | 1.1 | Core Nodes | 0 | 已加载 / 触发 |
| Microsoft Foundry Chat Model | @n8n/n8n-nodes-langchain.lmChatAzureOpenAi | 1.1 | AI | 0 | 已加载 |
| Microsoft Foundry Embeddings | @n8n/n8n-nodes-langchain.embeddingsAzureOpenAi | 1 | AI | 0 | 已加载 |
| Milvus Vector Store | @n8n/n8n-nodes-langchain.vectorStoreMilvus | 1.3 | AI | 0 | 已加载 |
| MiniMax | @n8n/n8n-nodes-langchain.minimax | 1.2 | AI | 5 | 已加载 |
| MiniMax Chat Model | @n8n/n8n-nodes-langchain.lmChatMinimax | 1.2 | AI | 0 | 已加载 |
| Mistral Cloud Chat Model | @n8n/n8n-nodes-langchain.lmChatMistralCloud | 1 | AI | 0 | 已加载 |
| Model Selector | @n8n/n8n-nodes-langchain.modelSelector | 1 | AI | 0 | 已加载 |
| MongoDB Atlas Vector Store | @n8n/n8n-nodes-langchain.vectorStoreMongoDBAtlas | 1.3 | AI | 0 | 已加载 |
| MongoDB Chat Memory | @n8n/n8n-nodes-langchain.memoryMongoDbChat | 1.1 | AI | 0 | 已加载 |
| Moonshot Kimi | @n8n/n8n-nodes-langchain.moonshot | 1 | AI | 4 | 已加载 |
| Moonshot Kimi Chat Model | @n8n/n8n-nodes-langchain.lmChatMoonshot | 1.1 | AI | 0 | 已加载 |
| Motorhead | @n8n/n8n-nodes-langchain.memoryMotorhead | 1.4 | AI | 0 | 已加载 / 隐藏 |
| MultiQuery Retriever | @n8n/n8n-nodes-langchain.retrieverMultiQuery | 1 | AI | 0 | 已加载 |
| NVIDIA Nemotron Chat Model | @n8n/n8n-nodes-langchain.lmChatNvidia | 1 | AI | 0 | 已加载 |
| NVIDIA Nemotron Embeddings | @n8n/n8n-nodes-langchain.embeddingsNvidia | 1 | AI | 0 | 已加载 |
| Ollama | @n8n/n8n-nodes-langchain.ollama | 1 | AI | 4 | 已加载 |
| Ollama Chat Model | @n8n/n8n-nodes-langchain.lmChatOllama | 1 | AI | 0 | 已加载 |
| Ollama Model | @n8n/n8n-nodes-langchain.lmOllama | 1 | AI | 0 | 已加载 |
| OpenAI | @n8n/n8n-nodes-langchain.openAi | 2.3 | AI | 16 | 已加载 |
| OpenAI Assistant | @n8n/n8n-nodes-langchain.openAiAssistant | 1.1 | AI | 0 | 已加载 / 隐藏 |
| OpenAI Chat Model | @n8n/n8n-nodes-langchain.lmChatOpenAi | 1.3 | AI | 0 | 已加载 |
| OpenAI Model | @n8n/n8n-nodes-langchain.lmOpenAi | 1 | AI | 0 | 已加载 / 隐藏 |
| OpenRouter Chat Model | @n8n/n8n-nodes-langchain.lmChatOpenRouter | 1 | AI | 0 | 已加载 |
| Oracle Database Vector Store | @n8n/n8n-nodes-langchain.vectorStoreOracleDBVector | 1.3 | AI | 0 | 已加载 |
| Pinecone Vector Store | @n8n/n8n-nodes-langchain.vectorStorePinecone | 1.3 | AI | 0 | 已加载 |
| Pinecone: Insert | @n8n/n8n-nodes-langchain.vectorStorePineconeInsert | 1 | AI | 0 | 已加载 / 隐藏 |
| Pinecone: Load | @n8n/n8n-nodes-langchain.vectorStorePineconeLoad | 1 | AI | 0 | 已加载 / 隐藏 |
| Postgres Chat Memory | @n8n/n8n-nodes-langchain.memoryPostgresChat | 1.4 | AI | 0 | 已加载 |
| Postgres PGVector Store | @n8n/n8n-nodes-langchain.vectorStorePGVector | 1.3 | AI | 0 | 已加载 |
| Qdrant Vector Store | @n8n/n8n-nodes-langchain.vectorStoreQdrant | 1.3 | AI | 0 | 已加载 |
| Question and Answer Chain | @n8n/n8n-nodes-langchain.chainRetrievalQa | 1.7 | AI | 0 | 已加载 |
| Qwen Cloud | @n8n/n8n-nodes-langchain.alibabaCloud | 1.1 | AI | 5 | 已加载 |
| Qwen Cloud Chat Model | @n8n/n8n-nodes-langchain.lmChatAlibabaCloud | 1.1 | AI | 0 | 已加载 |
| Recursive Character Text Splitter | @n8n/n8n-nodes-langchain.textSplitterRecursiveCharacterTextSplitter | 1 | AI | 0 | 已加载 |
| Redis Chat Memory | @n8n/n8n-nodes-langchain.memoryRedisChat | 1.6 | AI | 0 | 已加载 |
| Redis Vector Store | @n8n/n8n-nodes-langchain.vectorStoreRedis | 1.3 | AI | 0 | 已加载 |
| Reranker Cohere | @n8n/n8n-nodes-langchain.rerankerCohere | 1 | AI | 0 | 已加载 |
| SearXNG | @n8n/n8n-nodes-langchain.toolSearXng | 1 | AI | 0 | 已加载 |
| Sentiment Analysis | @n8n/n8n-nodes-langchain.sentimentAnalysis | 1.1 | AI | 0 | 已加载 |
| SerpApi (Google Search) | @n8n/n8n-nodes-langchain.toolSerpApi | 1 | AI | 0 | 已加载 / 隐藏 |
| Simple Memory | @n8n/n8n-nodes-langchain.memoryBufferWindow | 1.4 | AI | 0 | 已加载 |
| Simple Vector Store | @n8n/n8n-nodes-langchain.vectorStoreInMemory | 1.3 | AI | 0 | 已加载 |
| Structured Output Parser | @n8n/n8n-nodes-langchain.outputParserStructured | 1.3 | AI | 0 | 已加载 |
| Summarization Chain | @n8n/n8n-nodes-langchain.chainSummarization | 2.1 | AI | 0 | 已加载 |
| Supabase Vector Store | @n8n/n8n-nodes-langchain.vectorStoreSupabase | 1.3 | AI | 0 | 已加载 |
| Supabase: Insert | @n8n/n8n-nodes-langchain.vectorStoreSupabaseInsert | 1 | AI | 0 | 已加载 / 隐藏 |
| Supabase: Load | @n8n/n8n-nodes-langchain.vectorStoreSupabaseLoad | 1 | AI | 0 | 已加载 / 隐藏 |
| Text Classifier | @n8n/n8n-nodes-langchain.textClassifier | 1.1 | AI | 0 | 已加载 |
| Think Tool | @n8n/n8n-nodes-langchain.toolThink | 1.1 | AI | 0 | 已加载 |
| Token Splitter | @n8n/n8n-nodes-langchain.textSplitterTokenSplitter | 1 | AI | 0 | 已加载 |
| Tool Executor | @n8n/n8n-nodes-langchain.toolExecutor | 1 | Core Nodes | 0 | 已加载 / 隐藏 |
| Vector Store Question Answer Tool | @n8n/n8n-nodes-langchain.toolVectorStore | 1.1 | AI | 0 | 已加载 |
| Vector Store Retriever | @n8n/n8n-nodes-langchain.retrieverVectorStore | 1 | AI | 0 | 已加载 |
| Vercel AI Gateway Chat Model | @n8n/n8n-nodes-langchain.lmChatVercelAiGateway | 1 | AI | 0 | 已加载 |
| Weaviate Vector Store | @n8n/n8n-nodes-langchain.vectorStoreWeaviate | 1.3 | AI | 0 | 已加载 |
| Wikipedia | @n8n/n8n-nodes-langchain.toolWikipedia | 1 | AI | 0 | 已加载 |
| Wolfram\|Alpha | @n8n/n8n-nodes-langchain.toolWolframAlpha | 1 | AI | 0 | 已加载 |
| Workflow Retriever | @n8n/n8n-nodes-langchain.retrieverWorkflow | 1.1 | AI | 0 | 已加载 |
| xAI Grok Chat Model | @n8n/n8n-nodes-langchain.lmChatXAiGrok | 1 | AI | 0 | 已加载 |
| Xata | @n8n/n8n-nodes-langchain.memoryXata | 1.5 | AI | 0 | 已加载 |
| Zep | @n8n/n8n-nodes-langchain.memoryZep | 1.4 | AI | 0 | 已加载 / 隐藏 |
| Zep Vector Store | @n8n/n8n-nodes-langchain.vectorStoreZep | 1.3 | AI | 0 | 已加载 |
| Zep Vector Store: Insert | @n8n/n8n-nodes-langchain.vectorStoreZepInsert | 1 | AI | 0 | 已加载 / 隐藏 |
| Zep Vector Store: Load | @n8n/n8n-nodes-langchain.vectorStoreZepLoad | 1 | AI | 0 | 已加载 / 隐藏 |

## 运行时 Tool 变体（278）

| 名称 | 类型标识 | 版本 | 分类 | 静态操作数 | 状态 |
|---|---|---|---|---:|---|
| Action Network Tool | n8n-nodes-base.actionNetworkTool | 1 | AI | 30 | 已加载 |
| ActiveCampaign Tool | n8n-nodes-base.activeCampaignTool | 1 | AI | 60 | 已加载 |
| Adalo Tool | n8n-nodes-base.adaloTool | 1 | AI | 6 | 已加载 |
| Affinity Tool | n8n-nodes-base.affinityTool | 1 | AI | 16 | 已加载 |
| Agile CRM Tool | n8n-nodes-base.agileCrmTool | 1 | AI | 15 | 已加载 |
| Airtable Tool | n8n-nodes-base.airtableTool | 2.2 | AI | 8 | 已加载 |
| Airtop Tool | n8n-nodes-base.airtopTool | 1 | AI | 24 | 已加载 |
| AMQP Sender Tool | n8n-nodes-base.amqpTool | 1 | AI | 0 | 已加载 |
| Anthropic Tool | @n8n/n8n-nodes-langchain.anthropicTool | 1 | AI | 15 | 已加载 |
| APITemplate.io Tool | n8n-nodes-base.apiTemplateIoTool | 1 | AI | 6 | 已加载 |
| Asana Tool | n8n-nodes-base.asanaTool | 1 | AI | 29 | 已加载 |
| Autopilot Tool | n8n-nodes-base.autopilotTool | 1 | AI | 11 | 已加载 |
| AWS Lambda Tool | n8n-nodes-base.awsLambdaTool | 1 | AI | 2 | 已加载 |
| AWS S3 Tool | n8n-nodes-base.awsS3Tool | 2 | AI | 15 | 已加载 |
| AWS SES Tool | n8n-nodes-base.awsSesTool | 1 | AI | 16 | 已加载 |
| AWS SNS Tool | n8n-nodes-base.awsSnsTool | 1 | AI | 4 | 已加载 |
| AWS Textract Tool | n8n-nodes-base.awsTextractTool | 1 | AI | 2 | 已加载 |
| AWS Transcribe Tool | n8n-nodes-base.awsTranscribeTool | 1 | AI | 5 | 已加载 |
| BambooHR Tool | n8n-nodes-base.bambooHrTool | 1 | AI | 15 | 已加载 |
| Baserow Tool | n8n-nodes-base.baserowTool | 1.1 | AI | 9 | 已加载 |
| Beeminder Tool | n8n-nodes-base.beeminderTool | 1 | AI | 22 | 已加载 |
| Bitly Tool | n8n-nodes-base.bitlyTool | 1 | AI | 4 | 已加载 |
| Bitwarden Tool | n8n-nodes-base.bitwardenTool | 1 | AI | 19 | 已加载 |
| Brandfetch Tool | n8n-nodes-base.BrandfetchTool | 2 | AI | 4 | 已加载 |
| Brevo Tool | n8n-nodes-base.sendInBlueTool | 1.1 | AI | 19 | 已加载 |
| Bubble Tool | n8n-nodes-base.bubbleTool | 1 | AI | 5 | 已加载 |
| Chargebee Tool | n8n-nodes-base.chargebeeTool | 1 | AI | 15 | 已加载 |
| Chat | @n8n/n8n-nodes-langchain.chatHitlTool | 1.3 | AI | 0 | 已加载 |
| Chat Tool | @n8n/n8n-nodes-langchain.chatTool | 1.3 | AI | 2 | 已加载 |
| CircleCI Tool | n8n-nodes-base.circleCiTool | 1 | AI | 3 | 已加载 |
| Clearbit Tool | n8n-nodes-base.clearbitTool | 1 | AI | 5 | 已加载 |
| ClickUp Tool | n8n-nodes-base.clickUpTool | 1 | AI | 71 | 已加载 |
| Clockify Tool | n8n-nodes-base.clockifyTool | 1 | AI | 32 | 已加载 |
| Cloudflare Tool | n8n-nodes-base.cloudflareTool | 1 | AI | 5 | 已加载 |
| Cockpit Tool | n8n-nodes-base.cockpitTool | 1 | AI | 5 | 已加载 |
| Coda Tool | n8n-nodes-base.codaTool | 1.1 | AI | 18 | 已加载 |
| CoinGecko Tool | n8n-nodes-base.coinGeckoTool | 1 | AI | 9 | 已加载 |
| Compression Tool | n8n-nodes-base.compressionTool | 1.1 | AI | 2 | 已加载 |
| Confluence Tool | n8n-nodes-base.confluenceTool | 1 | AI | 22 | 已加载 |
| Contentful Tool | n8n-nodes-base.contentfulTool | 1 | AI | 7 | 已加载 |
| ConvertKit Tool | n8n-nodes-base.convertKitTool | 1 | AI | 20 | 已加载 |
| Copper Tool | n8n-nodes-base.copperTool | 1 | AI | 40 | 已加载 |
| CrateDB Tool | n8n-nodes-base.crateDbTool | 1.1 | AI | 3 | 已加载 |
| Crypto Tool | n8n-nodes-base.cryptoTool | 2 | AI | 0 | 已加载 |
| Currents Tool | n8n-nodes-base.currentsTool | 1 | AI | 30 | 已加载 |
| Customer.io Tool | n8n-nodes-base.customerIoTool | 1 | AI | 13 | 已加载 |
| Data table Tool | n8n-nodes-base.dataTableTool | 1.1 | AI | 12 | 已加载 |
| Databricks Tool | n8n-nodes-base.databricksTool | 1 | AI | 53 | 已加载 |
| Date & Time Tool | n8n-nodes-base.dateTimeTool | 2 | AI | 7 | 已加载 |
| DeepL Tool | n8n-nodes-base.deepLTool | 1 | AI | 2 | 已加载 |
| Demio Tool | n8n-nodes-base.demioTool | 1 | AI | 4 | 已加载 |
| DHL Tool | n8n-nodes-base.dhlTool | 1 | AI | 1 | 已加载 |
| Discord | n8n-nodes-base.discordHitlTool | 2 | AI | 0 | 已加载 |
| Discord Tool | n8n-nodes-base.discordTool | 2 | AI | 23 | 已加载 |
| Discourse Tool | n8n-nodes-base.discourseTool | 1 | AI | 21 | 已加载 |
| Drift Tool | n8n-nodes-base.driftTool | 1 | AI | 6 | 已加载 |
| Dropbox Tool | n8n-nodes-base.dropboxTool | 1 | AI | 14 | 已加载 |
| Dropcontact Tool | n8n-nodes-base.dropcontactTool | 1 | AI | 3 | 已加载 |
| E-goi Tool | n8n-nodes-base.egoiTool | 1 | AI | 4 | 已加载 |
| Elastic Security Tool | n8n-nodes-base.elasticSecurityTool | 1 | AI | 18 | 已加载 |
| Elasticsearch Tool | n8n-nodes-base.elasticsearchTool | 1 | AI | 11 | 已加载 |
| Emelia Tool | n8n-nodes-base.emeliaTool | 1 | AI | 9 | 已加载 |
| ERPNext Tool | n8n-nodes-base.erpNextTool | 1 | AI | 6 | 已加载 |
| Facebook Graph API Tool | n8n-nodes-base.facebookGraphApiTool | 1 | AI | 0 | 已加载 |
| FileMaker Tool | n8n-nodes-base.filemakerTool | 1 | AI | 0 | 已加载 |
| Freshdesk Tool | n8n-nodes-base.freshdeskTool | 1 | AI | 12 | 已加载 |
| Freshservice Tool | n8n-nodes-base.freshserviceTool | 1 | AI | 72 | 已加载 |
| Freshworks CRM Tool | n8n-nodes-base.freshworksCrmTool | 1 | AI | 40 | 已加载 |
| GetResponse Tool | n8n-nodes-base.getResponseTool | 1 | AI | 6 | 已加载 |
| Ghost Tool | n8n-nodes-base.ghostTool | 1 | AI | 9 | 已加载 |
| Git Tool | n8n-nodes-base.gitTool | 1.1 | AI | 15 | 已加载 |
| GitHub Tool | n8n-nodes-base.githubTool | 1.1 | AI | 57 | 已加载 |
| GitLab Tool | n8n-nodes-base.gitlabTool | 1 | AI | 23 | 已加载 |
| Gmail | n8n-nodes-base.gmailHitlTool | 2.2 | AI | 0 | 已加载 |
| Gmail Tool | n8n-nodes-base.gmailTool | 2.2 | AI | 26 | 已加载 |
| Gong Tool | n8n-nodes-base.gongTool | 1 | AI | 6 | 已加载 |
| Google Ads Tool | n8n-nodes-base.googleAdsTool | 1 | AI | 3 | 已加载 |
| Google Analytics Tool | n8n-nodes-base.googleAnalyticsTool | 2 | AI | 14 | 已加载 |
| Google BigQuery Tool | n8n-nodes-base.googleBigQueryTool | 2.1 | AI | 2 | 已加载 |
| Google Books Tool | n8n-nodes-base.googleBooksTool | 2 | AI | 12 | 已加载 |
| Google Business Profile Tool | n8n-nodes-base.googleBusinessProfileTool | 1 | AI | 11 | 已加载 |
| Google Calendar Tool | n8n-nodes-base.googleCalendarTool | 1.3 | AI | 8 | 已加载 |
| Google Chat | n8n-nodes-base.googleChatHitlTool | 1 | AI | 0 | 已加载 |
| Google Chat Tool | n8n-nodes-base.googleChatTool | 1 | AI | 12 | 已加载 |
| Google Cloud Firestore Tool | n8n-nodes-base.googleFirebaseCloudFirestoreTool | 1.1 | AI | 9 | 已加载 |
| Google Cloud Natural Language Tool | n8n-nodes-base.googleCloudNaturalLanguageTool | 1 | AI | 2 | 已加载 |
| Google Cloud Realtime Database Tool | n8n-nodes-base.googleFirebaseRealtimeDatabaseTool | 1 | AI | 6 | 已加载 |
| Google Cloud Storage Tool | n8n-nodes-base.googleCloudStorageTool | 1.1 | AI | 10 | 已加载 |
| Google Contacts Tool | n8n-nodes-base.googleContactsTool | 1 | AI | 6 | 已加载 |
| Google Docs Tool | n8n-nodes-base.googleDocsTool | 2 | AI | 4 | 已加载 |
| Google Drive Tool | n8n-nodes-base.googleDriveTool | 3 | AI | 21 | 已加载 |
| Google Gemini Tool | @n8n/n8n-nodes-langchain.googleGeminiTool | 1.2 | AI | 15 | 已加载 |
| Google Perspective Tool | n8n-nodes-base.googlePerspectiveTool | 1 | AI | 2 | 已加载 |
| Google Sheets Tool | n8n-nodes-base.googleSheetsTool | 4.7 | AI | 10 | 已加载 |
| Google Slides Tool | n8n-nodes-base.googleSlidesTool | 2 | AI | 8 | 已加载 |
| Google Tasks Tool | n8n-nodes-base.googleTasksTool | 1 | AI | 6 | 已加载 |
| Google Translate Tool | n8n-nodes-base.googleTranslateTool | 2 | AI | 2 | 已加载 |
| Google Workspace Admin Tool | n8n-nodes-base.gSuiteAdminTool | 1 | AI | 19 | 已加载 |
| Gotify Tool | n8n-nodes-base.gotifyTool | 1 | AI | 3 | 已加载 |
| GoToWebinar Tool | n8n-nodes-base.goToWebinarTool | 1 | AI | 28 | 已加载 |
| Grafana Tool | n8n-nodes-base.grafanaTool | 1 | AI | 20 | 已加载 |
| GraphQL Tool | n8n-nodes-base.graphqlTool | 1.1 | AI | 0 | 已加载 |
| Grist Tool | n8n-nodes-base.gristTool | 1 | AI | 6 | 已加载 |
| Hacker News Tool | n8n-nodes-base.hackerNewsTool | 1 | AI | 3 | 已加载 |
| HaloPSA Tool | n8n-nodes-base.haloPSATool | 1 | AI | 20 | 已加载 |
| Harvest Tool | n8n-nodes-base.harvestTool | 1 | AI | 61 | 已加载 |
| Help Scout Tool | n8n-nodes-base.helpScoutTool | 1 | AI | 17 | 已加载 |
| HighLevel Tool | n8n-nodes-base.highLevelTool | 2 | AI | 21 | 已加载 |
| Home Assistant Tool | n8n-nodes-base.homeAssistantTool | 1 | AI | 14 | 已加载 |
| HTTP Request Tool | n8n-nodes-base.httpRequestTool | 4.5 | AI | 0 | 已加载 |
| HubSpot Tool | n8n-nodes-base.hubspotTool | 2.2 | AI | 31 | 已加载 |
| Humantic AI Tool | n8n-nodes-base.humanticAiTool | 1 | AI | 3 | 已加载 |
| Hunter Tool | n8n-nodes-base.hunterTool | 1 | AI | 3 | 已加载 |
| Intercom Tool | n8n-nodes-base.intercomTool | 1 | AI | 18 | 已加载 |
| Invoice Ninja Tool | n8n-nodes-base.invoiceNinjaTool | 2 | AI | 38 | 已加载 |
| Iterable Tool | n8n-nodes-base.iterableTool | 1 | AI | 9 | 已加载 |
| Jenkins Tool | n8n-nodes-base.jenkinsTool | 1 | AI | 11 | 已加载 |
| Jina AI Tool | n8n-nodes-base.jinaAiTool | 1 | AI | 5 | 已加载 |
| Jira Software Tool | n8n-nodes-base.jiraTool | 1 | AI | 24 | 已加载 |
| JWT Tool | n8n-nodes-base.jwtTool | 1 | AI | 3 | 已加载 |
| Kafka Tool | n8n-nodes-base.kafkaTool | 2 | AI | 0 | 已加载 |
| Keap Tool | n8n-nodes-base.keapTool | 1 | AI | 36 | 已加载 |
| KoBoToolbox Tool | n8n-nodes-base.koBoToolboxTool | 1 | AI | 21 | 已加载 |
| Ldap Tool | n8n-nodes-base.ldapTool | 1 | AI | 6 | 已加载 |
| Lemlist Tool | n8n-nodes-base.lemlistTool | 2 | AI | 21 | 已加载 |
| Line Tool | n8n-nodes-base.lineTool | 1 | AI | 2 | 已加载 / 隐藏 |
| Linear Tool | n8n-nodes-base.linearTool | 1.1 | AI | 9 | 已加载 |
| LingvaNex Tool | n8n-nodes-base.lingvaNexTool | 1 | AI | 1 | 已加载 |
| LinkedIn Tool | n8n-nodes-base.linkedInTool | 1 | AI | 2 | 已加载 |
| LoneScale Tool | n8n-nodes-base.loneScaleTool | 1 | AI | 9 | 已加载 |
| Magento 2 Tool | n8n-nodes-base.magento2Tool | 1 | AI | 19 | 已加载 |
| Mailcheck Tool | n8n-nodes-base.mailcheckTool | 1 | AI | 1 | 已加载 |
| Mailchimp Tool | n8n-nodes-base.mailchimpTool | 1 | AI | 18 | 已加载 |
| MailerLite Tool | n8n-nodes-base.mailerLiteTool | 2 | AI | 4 | 已加载 |
| Mailgun Tool | n8n-nodes-base.mailgunTool | 1 | AI | 0 | 已加载 |
| Mailjet Tool | n8n-nodes-base.mailjetTool | 1 | AI | 5 | 已加载 |
| Mandrill Tool | n8n-nodes-base.mandrillTool | 1 | AI | 2 | 已加载 |
| Marketstack Tool | n8n-nodes-base.marketstackTool | 1 | AI | 3 | 已加载 |
| Matrix Tool | n8n-nodes-base.matrixTool | 1 | AI | 11 | 已加载 |
| Mattermost Tool | n8n-nodes-base.mattermostTool | 1 | AI | 23 | 已加载 |
| Mautic Tool | n8n-nodes-base.mauticTool | 1 | AI | 26 | 已加载 |
| Medium Tool | n8n-nodes-base.mediumTool | 1 | AI | 4 | 已加载 |
| Message an Agent Tool | n8n-nodes-base.messageAnAgentTool | 3.1 | AI | 0 | 已加载 |
| MessageBird Tool | n8n-nodes-base.messageBirdTool | 1 | AI | 2 | 已加载 |
| Metabase Tool | n8n-nodes-base.metabaseTool | 1 | AI | 14 | 已加载 |
| Microsoft Dataverse Tool | n8n-nodes-base.microsoftDataverseTool | 1 | AI | 7 | 已加载 |
| Microsoft Dynamics CRM Tool | n8n-nodes-base.microsoftDynamicsCrmTool | 1 | AI | 6 | 已加载 |
| Microsoft Entra ID Tool | n8n-nodes-base.microsoftEntraTool | 1 | AI | 14 | 已加载 |
| Microsoft Excel (OneDrive) Tool | n8n-nodes-base.microsoftExcelTool | 2.2 | AI | 17 | 已加载 |
| Microsoft Excel (SharePoint) Tool | n8n-nodes-base.microsoftExcelSharePointTool | 1 | AI | 21 | 已加载 |
| Microsoft Graph Security Tool | n8n-nodes-base.microsoftGraphSecurityTool | 1 | AI | 7 | 已加载 |
| Microsoft OneDrive Tool | n8n-nodes-base.microsoftOneDriveTool | 1.1 | AI | 18 | 已加载 |
| Microsoft Outlook | n8n-nodes-base.microsoftOutlookHitlTool | 2 | AI | 0 | 已加载 |
| Microsoft Outlook Tool | n8n-nodes-base.microsoftOutlookTool | 2 | AI | 46 | 已加载 |
| Microsoft SharePoint Tool | n8n-nodes-base.microsoftSharePointTool | 2 | AI | 14 | 已加载 |
| Microsoft SQL Tool | n8n-nodes-base.microsoftSqlTool | 1.2 | AI | 4 | 已加载 |
| Microsoft Teams | n8n-nodes-base.microsoftTeamsHitlTool | 2 | AI | 0 | 已加载 |
| Microsoft Teams Tool | n8n-nodes-base.microsoftTeamsTool | 2 | AI | 43 | 已加载 |
| Microsoft To Do Tool | n8n-nodes-base.microsoftToDoTool | 1 | AI | 18 | 已加载 |
| MiniMax Tool | @n8n/n8n-nodes-langchain.minimaxTool | 1.2 | AI | 5 | 已加载 |
| MISP Tool | n8n-nodes-base.mispTool | 1 | AI | 55 | 已加载 |
| Mistral AI Tool | n8n-nodes-base.mistralAiTool | 1 | AI | 2 | 已加载 |
| Mocean Tool | n8n-nodes-base.moceanTool | 1 | AI | 1 | 已加载 |
| Monday.com Tool | n8n-nodes-base.mondayComTool | 2 | AI | 44 | 已加载 |
| MongoDB Tool | n8n-nodes-base.mongoDbTool | 1.5 | AI | 11 | 已加载 |
| Monica CRM Tool | n8n-nodes-base.monicaCrmTool | 1 | AI | 52 | 已加载 |
| Moonshot Kimi Tool | @n8n/n8n-nodes-langchain.moonshotTool | 1 | AI | 4 | 已加载 |
| MQTT Tool | n8n-nodes-base.mqttTool | 1 | AI | 0 | 已加载 |
| MSG91 Tool | n8n-nodes-base.msg91Tool | 1 | AI | 1 | 已加载 |
| MySQL Tool | n8n-nodes-base.mySqlTool | 2.5 | AI | 6 | 已加载 |
| NASA Tool | n8n-nodes-base.nasaTool | 1 | AI | 40 | 已加载 |
| Netlify Tool | n8n-nodes-base.netlifyTool | 1 | AI | 9 | 已加载 |
| Nextcloud Tool | n8n-nodes-base.nextCloudTool | 1 | AI | 20 | 已加载 |
| NocoDB Tool | n8n-nodes-base.nocoDbTool | 4 | AI | 13 | 已加载 |
| Notion Tool | n8n-nodes-base.notionTool | 3 | AI | 23 | 已加载 |
| Npm Tool | n8n-nodes-base.npmTool | 1 | AI | 7 | 已加载 |
| Odoo Tool | n8n-nodes-base.odooTool | 2 | AI | 24 | 已加载 |
| Okta Tool | n8n-nodes-base.oktaTool | 1 | AI | 6 | 已加载 |
| Ollama Tool | @n8n/n8n-nodes-langchain.ollamaTool | 1 | AI | 4 | 已加载 |
| One Simple API Tool | n8n-nodes-base.oneSimpleApiTool | 1 | AI | 10 | 已加载 |
| Onfleet Tool | n8n-nodes-base.onfleetTool | 1 | AI | 37 | 已加载 |
| OpenThesaurus Tool | n8n-nodes-base.openThesaurusTool | 1 | AI | 1 | 已加载 |
| OpenWeatherMap Tool | n8n-nodes-base.openWeatherMapTool | 1 | AI | 3 | 已加载 |
| Oracle Database Tool | n8n-nodes-base.oracleDatabaseTool | 1.1 | AI | 6 | 已加载 |
| Oura Tool | n8n-nodes-base.ouraTool | 1 | AI | 6 | 已加载 |
| Paddle Tool | n8n-nodes-base.paddleTool | 1 | AI | 9 | 已加载 |
| PagerDuty Tool | n8n-nodes-base.pagerDutyTool | 1 | AI | 13 | 已加载 |
| Peekalink Tool | n8n-nodes-base.peekalinkTool | 1 | AI | 3 | 已加载 |
| Perplexity Tool | n8n-nodes-base.perplexityTool | 3 | AI | 4 | 已加载 |
| Phantombuster Tool | n8n-nodes-base.phantombusterTool | 1 | AI | 7 | 已加载 |
| Philips Hue Tool | n8n-nodes-base.philipsHueTool | 1 | AI | 5 | 已加载 |
| Pipedrive Tool | n8n-nodes-base.pipedriveTool | 2 | AI | 58 | 已加载 |
| Plivo Tool | n8n-nodes-base.plivoTool | 1 | AI | 3 | 已加载 |
| PostBin Tool | n8n-nodes-base.postBinTool | 1 | AI | 6 | 已加载 |
| Postgres Tool | n8n-nodes-base.postgresTool | 2.8 | AI | 6 | 已加载 |
| PostHog Tool | n8n-nodes-base.postHogTool | 1 | AI | 9 | 已加载 |
| ProfitWell Tool | n8n-nodes-base.profitWellTool | 1 | AI | 2 | 已加载 |
| Pushbullet Tool | n8n-nodes-base.pushbulletTool | 1 | AI | 5 | 已加载 |
| Pushcut Tool | n8n-nodes-base.pushcutTool | 1 | AI | 1 | 已加载 |
| Pushover Tool | n8n-nodes-base.pushoverTool | 1 | AI | 2 | 已加载 |
| QuestDB Tool | n8n-nodes-base.questDbTool | 1.1 | AI | 2 | 已加载 |
| Quick Base Tool | n8n-nodes-base.quickbaseTool | 1 | AI | 10 | 已加载 |
| QuickBooks Online Tool | n8n-nodes-base.quickbooksTool | 1 | AI | 52 | 已加载 |
| QuickChart Tool | n8n-nodes-base.quickChartTool | 1 | AI | 0 | 已加载 |
| Qwen Cloud Tool | @n8n/n8n-nodes-langchain.alibabaCloudTool | 1.1 | AI | 5 | 已加载 |
| RabbitMQ Tool | n8n-nodes-base.rabbitmqTool | 1.2 | AI | 4 | 已加载 |
| Raindrop Tool | n8n-nodes-base.raindropTool | 1 | AI | 17 | 已加载 |
| Reddit Tool | n8n-nodes-base.redditTool | 1 | AI | 18 | 已加载 |
| Redis Tool | n8n-nodes-base.redisTool | 1 | AI | 10 | 已加载 |
| RocketChat Tool | n8n-nodes-base.rocketchatTool | 1 | AI | 7 | 已加载 |
| RSS Read Tool | n8n-nodes-base.rssFeedReadTool | 1.2 | AI | 0 | 已加载 |
| Rundeck Tool | n8n-nodes-base.rundeckTool | 1.1 | AI | 3 | 已加载 |
| S3 Tool | n8n-nodes-base.s3Tool | 1 | AI | 12 | 已加载 |
| Salesforce Tool | n8n-nodes-base.salesforceTool | 1.1 | AI | 82 | 已加载 |
| Salesmate Tool | n8n-nodes-base.salesmateTool | 1 | AI | 15 | 已加载 |
| SeaTable Tool | n8n-nodes-base.seaTableTool | 2 | AI | 16 | 已加载 |
| SecurityScorecard Tool | n8n-nodes-base.securityScorecardTool | 1 | AI | 19 | 已加载 |
| Segment Tool | n8n-nodes-base.segmentTool | 1 | AI | 7 | 已加载 |
| Send Email | n8n-nodes-base.emailSendHitlTool | 2.1 | AI | 0 | 已加载 |
| Send Email Tool | n8n-nodes-base.emailSendTool | 2.1 | AI | 2 | 已加载 |
| SendGrid Tool | n8n-nodes-base.sendGridTool | 1 | AI | 13 | 已加载 |
| Sendy Tool | n8n-nodes-base.sendyTool | 1 | AI | 6 | 已加载 |
| Sentry.io Tool | n8n-nodes-base.sentryIoTool | 1 | AI | 31 | 已加载 |
| ServiceNow Tool | n8n-nodes-base.serviceNowTool | 1 | AI | 35 | 已加载 |
| seven Tool | n8n-nodes-base.sms77Tool | 1 | AI | 4 | 已加载 |
| Shopify Tool | n8n-nodes-base.shopifyTool | 1 | AI | 12 | 已加载 |
| SIGNL4 Tool | n8n-nodes-base.signl4Tool | 1 | AI | 2 | 已加载 |
| Slack | n8n-nodes-base.slackHitlTool | 2.7 | AI | 0 | 已加载 |
| Slack Tool | n8n-nodes-base.slackTool | 2.7 | AI | 48 | 已加载 |
| Snowflake Tool | n8n-nodes-base.snowflakeTool | 1.1 | AI | 4 | 已加载 |
| Splunk Tool | n8n-nodes-base.splunkTool | 2 | AI | 20 | 已加载 |
| Spotify Tool | n8n-nodes-base.spotifyTool | 1 | AI | 37 | 已加载 |
| Stackby Tool | n8n-nodes-base.stackbyTool | 1 | AI | 4 | 已加载 |
| Storyblok Tool | n8n-nodes-base.storyblokTool | 1 | AI | 7 | 已加载 |
| Strapi Tool | n8n-nodes-base.strapiTool | 1 | AI | 6 | 已加载 |
| Strava Tool | n8n-nodes-base.stravaTool | 1.1 | AI | 10 | 已加载 |
| Stripe Tool | n8n-nodes-base.stripeTool | 1 | AI | 28 | 已加载 |
| Supabase Tool | n8n-nodes-base.supabaseTool | 1 | AI | 6 | 已加载 |
| SyncroMSP Tool | n8n-nodes-base.syncroMspTool | 1 | AI | 20 | 已加载 |
| Taiga Tool | n8n-nodes-base.taigaTool | 1 | AI | 20 | 已加载 |
| Tapfiliate Tool | n8n-nodes-base.tapfiliateTool | 1 | AI | 12 | 已加载 |
| Telegram | n8n-nodes-base.telegramHitlTool | 1.2 | AI | 0 | 已加载 |
| Telegram Tool | n8n-nodes-base.telegramTool | 1.2 | AI | 27 | 已加载 |
| TheHive 5 Tool | n8n-nodes-base.theHiveProjectTool | 1 | AI | 56 | 已加载 |
| TheHive Tool | n8n-nodes-base.theHiveTool | 1 | AI | 5 | 已加载 |
| TimescaleDB Tool | n8n-nodes-base.timescaleDbTool | 1.1 | AI | 3 | 已加载 |
| Todoist Tool | n8n-nodes-base.todoistTool | 2.2 | AI | 36 | 已加载 |
| TOTP Tool | n8n-nodes-base.totpTool | 1 | AI | 1 | 已加载 |
| TravisCI Tool | n8n-nodes-base.travisCiTool | 1 | AI | 5 | 已加载 |
| Trello Tool | n8n-nodes-base.trelloTool | 1 | AI | 49 | 已加载 |
| Twake Tool | n8n-nodes-base.twakeTool | 1 | AI | 2 | 已加载 |
| Twilio Tool | n8n-nodes-base.twilioTool | 1 | AI | 4 | 已加载 |
| Twist Tool | n8n-nodes-base.twistTool | 1 | AI | 26 | 已加载 |
| Unleashed Software Tool | n8n-nodes-base.unleashedSoftwareTool | 1.1 | AI | 3 | 已加载 |
| Uplead Tool | n8n-nodes-base.upleadTool | 1 | AI | 2 | 已加载 |
| uProc Tool | n8n-nodes-base.uprocTool | 1 | AI | 0 | 已加载 |
| UptimeRobot Tool | n8n-nodes-base.uptimeRobotTool | 1 | AI | 26 | 已加载 |
| urlscan.io Tool | n8n-nodes-base.urlScanIoTool | 1 | AI | 4 | 已加载 |
| Venafi TLS Protect Cloud Tool | n8n-nodes-base.venafiTlsProtectCloudTool | 1 | AI | 10 | 已加载 |
| Venafi TLS Protect Datacenter Tool | n8n-nodes-base.venafiTlsProtectDatacenterTool | 1 | AI | 9 | 已加载 |
| Vero Tool | n8n-nodes-base.veroTool | 1 | AI | 8 | 已加载 |
| Vonage Tool | n8n-nodes-base.vonageTool | 1 | AI | 1 | 已加载 |
| Webex by Cisco Tool | n8n-nodes-base.ciscoWebexTool | 1 | AI | 12 | 已加载 |
| Webflow Tool | n8n-nodes-base.webflowTool | 2 | AI | 5 | 已加载 |
| Wekan Tool | n8n-nodes-base.wekanTool | 1 | AI | 30 | 已加载 |
| WhatsApp Business Cloud | n8n-nodes-base.whatsAppHitlTool | 1.1 | AI | 0 | 已加载 |
| WhatsApp Business Cloud Tool | n8n-nodes-base.whatsAppTool | 1.1 | AI | 6 | 已加载 |
| WooCommerce Tool | n8n-nodes-base.wooCommerceTool | 1 | AI | 18 | 已加载 |
| Wordpress Tool | n8n-nodes-base.wordpressTool | 1 | AI | 15 | 已加载 |
| X (Formerly Twitter) Tool | n8n-nodes-base.twitterTool | 2 | AI | 12 | 已加载 |
| Xero Tool | n8n-nodes-base.xeroTool | 1 | AI | 10 | 已加载 |
| Yourls Tool | n8n-nodes-base.yourlsTool | 1 | AI | 3 | 已加载 |
| YouTube Tool | n8n-nodes-base.youTubeTool | 1 | AI | 25 | 已加载 |
| Zammad Tool | n8n-nodes-base.zammadTool | 1 | AI | 21 | 已加载 |
| Zendesk Tool | n8n-nodes-base.zendeskTool | 1 | AI | 27 | 已加载 |
| Zoho CRM Tool | n8n-nodes-base.zohoCrmTool | 1 | AI | 71 | 已加载 |
| Zoom Tool | n8n-nodes-base.zoomTool | 1 | AI | 6 | 已加载 |
| Zulip Tool | n8n-nodes-base.zulipTool | 1 | AI | 16 | 已加载 |

## 运行时 MCP 目录（78）

| 名称 | 类型标识 | 版本 | 分类 | 静态操作数 | 状态 |
|---|---|---|---|---:|---|
| Airtable MCP | @n8n/mcp-registry.airtable | 1.1 | AI | 0 | 已加载 |
| Alchemy MCP | @n8n/mcp-registry.alchemy | 1.1 | AI | 0 | 已加载 |
| Altmetric MCP | @n8n/mcp-registry.altmetric | 1.1 | AI | 0 | 已加载 |
| Amplitude MCP | @n8n/mcp-registry.amplitude | 1.1 | AI | 0 | 已加载 |
| Apify MCP | @n8n/mcp-registry.apify | 1.1 | AI | 0 | 已加载 |
| Atlassian MCP | @n8n/mcp-registry.atlassian | 1.1 | AI | 0 | 已加载 |
| Avo MCP | @n8n/mcp-registry.avo | 1.1 | AI | 0 | 已加载 |
| Axiom MCP | @n8n/mcp-registry.axiom | 1.1 | AI | 0 | 已加载 |
| Bitrix24 MCP | @n8n/mcp-registry.bitrix_24 | 1.1 | AI | 0 | 已加载 |
| Brandfetch MCP | @n8n/mcp-registry.brandfetch | 1.1 | AI | 0 | 已加载 |
| CData Connect AI MCP | @n8n/mcp-registry.cDataConnectAi | 1.1 | AI | 0 | 已加载 |
| Cello MCP | @n8n/mcp-registry.cello | 1.1 | AI | 0 | 已加载 |
| ClickUp MCP | @n8n/mcp-registry.clickUp | 1.1 | AI | 0 | 已加载 |
| Close MCP | @n8n/mcp-registry.close | 1.1 | AI | 0 | 已加载 |
| Composio MCP | @n8n/mcp-registry.composio | 1.1 | AI | 0 | 已加载 |
| CryptoQuant MCP | @n8n/mcp-registry.cryptoQuant | 1.1 | AI | 0 | 已加载 |
| Cypress Cloud MCP | @n8n/mcp-registry.cypressCloud | 1.1 | AI | 0 | 已加载 |
| Databricks Genie One MCP | @n8n/mcp-registry.databricksGenie | 1.1 | AI | 0 | 已加载 |
| Egnyte Remote MCP | @n8n/mcp-registry.egnyteRemote | 1.1 | AI | 0 | 已加载 |
| Exa MCP | @n8n/mcp-registry.exa | 1.1 | AI | 0 | 已加载 |
| Fibery MCP | @n8n/mcp-registry.fibery | 1.1 | AI | 0 | 已加载 |
| Fingerprint MCP | @n8n/mcp-registry.fingerprint | 1.1 | AI | 0 | 已加载 |
| FLUX MCP | @n8n/mcp-registry.flux | 1.1 | AI | 0 | 已加载 |
| GitHub MCP | @n8n/mcp-registry.gitHub | 1.1 | AI | 0 | 已加载 |
| GitLab MCP | @n8n/mcp-registry.gitLab | 1.1 | AI | 0 | 已加载 |
| Grafana MCP | @n8n/mcp-registry.grafana | 1.1 | AI | 0 | 已加载 |
| Gtmetrix MCP | @n8n/mcp-registry.gtmetrix | 1.1 | AI | 0 | 已加载 |
| Guru Remote MCP | @n8n/mcp-registry.guruRemote | 1.1 | AI | 0 | 已加载 |
| Hackle MCP | @n8n/mcp-registry.hackle | 1.1 | AI | 0 | 已加载 |
| Himalayas Remote Jobs MCP | @n8n/mcp-registry.himalayasRemoteJobs | 1.1 | AI | 0 | 已加载 |
| Honeycomb MCP | @n8n/mcp-registry.honeycomb | 1.1 | AI | 0 | 已加载 |
| Hugging Face MCP | @n8n/mcp-registry.huggingFace | 1.1 | AI | 0 | 已加载 |
| Ivisa MCP | @n8n/mcp-registry.ivisa | 1.1 | AI | 0 | 已加载 |
| Jotform MCP | @n8n/mcp-registry.jotform | 1.1 | AI | 0 | 已加载 |
| Jumpcloud MCP | @n8n/mcp-registry.jumpcloud | 1.1 | AI | 0 | 已加载 |
| Kajabi MCP | @n8n/mcp-registry.kajabi | 1.1 | AI | 0 | 已加载 |
| Linear MCP | @n8n/mcp-registry.linear | 1.1 | AI | 0 | 已加载 |
| Lucid MCP | @n8n/mcp-registry.lucid | 1.1 | AI | 0 | 已加载 |
| Lusha MCP | @n8n/mcp-registry.lusha | 1.1 | AI | 0 | 已加载 |
| Mapbox MCP | @n8n/mcp-registry.mapbox | 1.1 | AI | 0 | 已加载 |
| Miro MCP | @n8n/mcp-registry.miro | 1.1 | AI | 0 | 已加载 |
| monday.com MCP | @n8n/mcp-registry.mondayCom | 1.1 | AI | 0 | 已加载 |
| Mux MCP | @n8n/mcp-registry.mux | 1.1 | AI | 0 | 已加载 |
| New Relic MCP | @n8n/mcp-registry.newRelic | 1.1 | AI | 0 | 已加载 |
| Notion MCP | @n8n/mcp-registry.notion | 1.1 | AI | 0 | 已加载 |
| Onlyoffice MCP | @n8n/mcp-registry.onlyoffice | 1.1 | AI | 0 | 已加载 |
| Open Video MCP | @n8n/mcp-registry.openVideo | 1.1 | AI | 0 | 已加载 |
| OpenAgenda MCP | @n8n/mcp-registry.openAgenda | 1.1 | AI | 0 | 已加载 |
| OpusClip MCP | @n8n/mcp-registry.opusClip | 1.1 | AI | 0 | 已加载 |
| PayPal MCP | @n8n/mcp-registry.payPal | 1.1 | AI | 0 | 已加载 |
| Pipedrive MCP | @n8n/mcp-registry.pipedrive | 1.1 | AI | 0 | 已加载 |
| PostHog MCP | @n8n/mcp-registry.postHog | 1.1 | AI | 0 | 已加载 |
| Postman MCP | @n8n/mcp-registry.postman | 1.1 | AI | 0 | 已加载 |
| Prisma MCP | @n8n/mcp-registry.prisma | 1.1 | AI | 0 | 已加载 |
| Quicknode MCP | @n8n/mcp-registry.quicknode | 1.1 | AI | 0 | 已加载 |
| Rackspace Spot MCP | @n8n/mcp-registry.rackspaceSpot | 1.1 | AI | 0 | 已加载 |
| Railway MCP | @n8n/mcp-registry.railway | 1.1 | AI | 0 | 已加载 |
| Roboflow (Official) MCP | @n8n/mcp-registry.roboflowOfficial | 1.1 | AI | 0 | 已加载 |
| Sanity MCP | @n8n/mcp-registry.sanity | 1.1 | AI | 0 | 已加载 |
| SendPulse MCP | @n8n/mcp-registry.sendPulse | 1.1 | AI | 0 | 已加载 |
| Serpstat MCP | @n8n/mcp-registry.serpstat | 1.1 | AI | 0 | 已加载 |
| SmartBear MCP | @n8n/mcp-registry.smartBear | 1.1 | AI | 0 | 已加载 |
| Snitcher MCP | @n8n/mcp-registry.snitcher | 1.1 | AI | 0 | 已加载 |
| StackOne MCP | @n8n/mcp-registry.stackone | 1.1 | AI | 0 | 已加载 |
| Stripe MCP | @n8n/mcp-registry.stripe | 1.1 | AI | 0 | 已加载 |
| Supabase MCP | @n8n/mcp-registry.supabase | 1.1 | AI | 0 | 已加载 |
| Tavily MCP | @n8n/mcp-registry.tavily | 1.1 | AI | 0 | 已加载 |
| Tenderly MCP | @n8n/mcp-registry.tenderly | 1.1 | AI | 0 | 已加载 |
| Tolstoy Library MCP | @n8n/mcp-registry.tolstoyLibrary | 1.1 | AI | 0 | 已加载 |
| Trello MCP | @n8n/mcp-registry.trello | 1.1 | AI | 0 | 已加载 |
| Unstoppable Domains MCP | @n8n/mcp-registry.unstoppableDomains | 1.1 | AI | 0 | 已加载 |
| UserGuiding MCP | @n8n/mcp-registry.userGuiding | 1.1 | AI | 0 | 已加载 |
| VEED AI Video Generator MCP | @n8n/mcp-registry.veedAiVideoGenerator | 1.1 | AI | 0 | 已加载 |
| Webflow MCP | @n8n/mcp-registry.webflow | 1.1 | AI | 0 | 已加载 |
| Wix MCP | @n8n/mcp-registry.wix | 1.1 | AI | 0 | 已加载 |
| Wordlift MCP | @n8n/mcp-registry.wordlift | 1.1 | AI | 0 | 已加载 |
| You.com Web Access & AI MCP | @n8n/mcp-registry.youComWebAccessAi | 1.1 | AI | 0 | 已加载 |
| Zigpoll MCP | @n8n/mcp-registry.zigpoll | 1.1 | AI | 0 | 已加载 |

## 复现

使用 scripts/inventory-n8n.mjs，指定安装的 node_modules 及已登录实例导出的 /types/nodes.json、/types/credentials.json。脚本只读输入；原始快照留在仓库外，不写入文档目录。

此次只完成目录提取。下一阶段将这些目录事实与 Ferry 原有能力清单逐项映射；该进度只在 MEMORY.md 维护，不在此另建进度表。
