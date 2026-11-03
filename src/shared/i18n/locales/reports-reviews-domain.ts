const locales = ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"];
const rows = `cancelling|Cancelling|正在取消|正在取消|キャンセル中|취소 중|Sedang dibatalkan
cancelled|Cancelled|已取消|已取消|キャンセル済み|취소됨|Dibatalkan
cancelRun|Cancel run|取消执行|取消執行|実行をキャンセル|실행 취소|Batalkan pelaksanaan
cancelRunHelp|Cancel this active review and its pipeline execution.|取消此活动走查及其流水线执行。|取消此進行中的審查及其流水線執行。|この進行中のレビューとパイプライン実行をキャンセルします。|이 활성 검토 및 파이프라인 실행을 취소합니다.|Batalkan semakan aktif ini dan pelaksanaan saluran paipnya.
reviewEvidence|Test and review evidence|测试与走查证据|測試與審查證據|テストとレビューの証拠|테스트 및 검토 증거|Bukti ujian dan semakan
noReviewEvidence|No execution evidence for this report scope and period.|此报告范围与期间暂无执行证据。|此報告範圍與期間尚無執行證據。|このレポートの範囲と期間には実行の証拠がありません。|이 보고서 범위와 기간에는 실행 증거가 없습니다.|Tiada bukti pelaksanaan untuk skop dan tempoh laporan ini.
evidence.permission|Code review read permission is required.|需要代码走查读取权限。|需要程式碼審查讀取權限。|コードレビューの読み取り権限が必要です。|코드 검토 읽기 권한이 필요합니다.|Kebenaran membaca semakan kod diperlukan.
evidence.credential|Repository credential unavailable.|仓库凭据不可用。|儲存庫憑證無法使用。|リポジトリの認証情報を利用できません。|저장소 자격 증명을 사용할 수 없습니다.|Kelayakan repositori tidak tersedia.
evidence.authorization|Repository authorization unavailable.|仓库授权不可用。|儲存庫授權無法使用。|リポジトリの権限を利用できません。|저장소 권한을 사용할 수 없습니다.|Kebenaran repositori tidak tersedia.
evidence.total|Review executions|走查执行数|審查執行數|レビュー実行数|검토 실행 수|Pelaksanaan semakan
evidence.tests_passed|Tests passed|测试通过数|測試通過數|テスト合格数|테스트 통과 수|Ujian lulus
evidence.tests_failed|Tests failed|测试失败数|測試失敗數|テスト失敗数|테스트 실패 수|Ujian gagal
evidence.findings|Recorded findings|已记录问题数|已記錄問題數|記録された検出事項|기록된 발견 사항|Penemuan direkodkan
addRepository|Add repository|添加仓库|新增儲存庫|リポジトリを追加|저장소 추가|Tambah repositori
exportReport|Export report|导出报告|匯出報告|レポートをエクスポート|보고서 내보내기|Eksport laporan
createReport|Create report|创建报告|建立報告|レポートを作成|보고서 만들기|Cipta laporan
waiting|Waiting for Runner|等待 Runner|等待 Runner|Runner を待機中|Runner 대기 중|Menunggu Runner
launching|Launching pipeline|正在启动流水线|正在啟動流水線|パイプラインを起動中|파이프라인 시작 중|Melancarkan saluran paip
periodStartRead|Period start|期间开始|期間開始|期間開始|기간 시작|Mula tempoh
periodEnd|Period end|期间结束|期間結束|期間終了|기간 종료|Akhir tempoh
identity_version|Author mapping version|作者映射版本|作者映射版本|作者マッピングのバージョン|작성자 매핑 버전|Versi pemetaan pengarang
repository_scope_version|Repository scope version|仓库范围版本|儲存庫範圍版本|リポジトリ範囲のバージョン|저장소 범위 버전|Versi skop repositori
trigger_error|Trigger error|触发错误|觸發錯誤|トリガーエラー|트리거 오류|Ralat pencetus
ai_concurrency|Concurrent AI audits|AI 审计并发数|AI 審計並行數|AI 監査の同時実行数|동시 AI 감사 수|Audit AI serentak
unavailable|Result unavailable|结果不可用|結果無法取得|結果を取得できません|결과 없음|Keputusan tidak tersedia
testsFailed|Tests failed|测试失败|測試失敗|テスト失敗|테스트 실패|Ujian gagal
total|Total tests|测试总数|測試總數|テスト総数|전체 테스트 수|Jumlah ujian
duration_ms|Duration (ms)|耗时（毫秒）|耗時（毫秒）|所要時間（ミリ秒）|소요 시간 (밀리초)|Tempoh (ms)
reports|Insight reports|洞察报告|洞察報告|インサイトレポート|인사이트 보고서|Laporan cerapan
reviews|Code reviews|代码走查|程式碼審查|コードレビュー|코드 검토|Semakan kod
name|Name|名称|名稱|名前|이름|Nama
source|Source insight|来源洞察|來源洞察|ソースインサイト|원본 인사이트|Sumber cerapan
periods|Report periods|报告周期|報告週期|レポート期間|보고 기간|Tempoh laporan
day|Daily|日报|日報|日次|일간|Harian
week|Weekly|周报|週報|週次|주간|Mingguan
month|Monthly|月报|月報|月次|월간|Bulanan
quarter|Quarterly|季报|季報|四半期|분기|Suku tahunan
year|Yearly|年报|年報|年次|연간|Tahunan
dimension|Analysis dimension|分析维度|分析維度|分析軸|분석 기준|Dimensi analisis
person|People|按人员|依人員|担当者別|사용자별|Mengikut individu
application|Applications|按应用|依應用|アプリ別|애플리케이션별|Mengikut aplikasi
people|Contributors|贡献人员|貢獻人員|貢献者|기여자|Penyumbang
search|Search people and aliases|搜索人员与别名|搜尋人員與別名|担当者と別名を検索|사용자 및 별칭 검색|Cari individu dan alias
applications|Application repository scope|应用仓库范围|應用儲存庫範圍|アプリのリポジトリ範囲|앱 저장소 범위|Skop repositori aplikasi
aliases|Manage author aliases|管理作者昵称映射|管理作者暱稱映射|作者の別名を管理|작성자 별칭 관리|Urus alias pengarang
timezone|Report timezone|报告时区|報告時區|レポートのタイムゾーン|보고서 시간대|Zon waktu laporan
enabled|Enabled|启用|啟用|有効|사용|Diaktifkan
view|View|查看|檢視|表示|보기|Lihat
edit|Edit|编辑|編輯|編集|편집|Sunting
delete|Delete|删除|刪除|削除|삭제|Padam
create|Create|创建|建立|作成|만들기|Cipta
generate|Generate report|生成报告|產生報告|レポートを生成|보고서 생성|Jana laporan
period|Period|周期|週期|期間|기간|Tempoh
periodStart|Historical period start (optional)|历史期间起始日（可选）|歷史期間起始日（選填）|過去の期間開始日（任意）|과거 기간 시작일 (선택)|Tarikh mula sejarah (pilihan)
configuration|Configuration|配置|設定|設定|설정|Konfigurasi
overview|Overview|概览|概覽|概要|개요|Gambaran keseluruhan
history|Run history|运行记录|執行記錄|実行履歴|실행 기록|Sejarah pelaksanaan
findings|Findings|问题列表|問題清單|検出事項|발견 사항|Penemuan
state|Status|状态|狀態|状態|상태|Status
version|Configuration version|配置版本|設定版本|設定バージョン|설정 버전|Versi konfigurasi
next|Next generation|下次生成|下次產生|次回生成|다음 생성|Penjanaan seterusnya
last|Last generation|最近生成|最近產生|前回生成|최근 생성|Penjanaan terakhir
objects|Selected subjects|所选对象数|所選對象數|選択対象数|선택 대상 수|Bilangan subjek
loading|Loading…|加载中…|載入中…|読み込み中…|로딩 중…|Memuatkan…
empty|No records yet|暂无记录|尚無記錄|記録はまだありません|기록이 없습니다|Belum ada rekod
coverage|Data coverage|数据覆盖|資料涵蓋|データカバレッジ|데이터 범위|Liputan data
complete|Complete|完整|完整|完全|완료|Lengkap
incomplete|Incomplete data|数据不完整|資料不完整|データ不完全|불완전한 데이터|Data tidak lengkap
commits|Commits|提交数|提交數|コミット数|커밋 수|Komit
additions|Lines added|新增行数|新增行數|追加行数|추가 행|Baris ditambah
deletions|Lines removed|删除行数|刪除行數|削除行数|삭제 행|Baris dibuang
effectives|Effective changes|有效变更|有效變更|有効な変更|유효 변경|Perubahan berkesan
active_days|Active days|活跃天数|活躍天數|活動日数|활동 일수|Hari aktif
repository_count|Repositories|仓库数|儲存庫數|リポジトリ数|저장소 수|Repositori
contributor_count|Contributors|贡献人数|貢獻人數|貢献者数|기여자 수|Penyumbang
trend|Period activity trend|周期活动趋势|週期活動趨勢|期間の活動推移|기간 활동 추이|Trend aktiviti tempoh
comparison|Previous period|上期对比|上期比較|前期間比較|이전 기간|Tempoh sebelumnya
contributions|Contribution details|贡献明细|貢獻明細|貢献詳細|기여 상세|Butiran sumbangan
preview|Current period preview (not closed)|当前周期预览（未结期）|目前週期預覽（尚未結期）|現在の期間のプレビュー（未確定）|현재 기간 미리보기 (미마감)|Pratonton tempoh semasa (belum tutup)
evidence|Activity evidence; not a project completion percentage.|研发活动证据，不代表项目完成百分比。|研發活動證據，不代表專案完成百分比。|開発活動の証拠です。プロジェクト完了率ではありません。|개발 활동 증거이며 프로젝트 완료율이 아닙니다.|Bukti aktiviti pembangunan, bukan peratus siap projek.
repositories|Repository scope|仓库范围|儲存庫範圍|リポジトリ範囲|저장소 범위|Skop repositori
repository|Repository (optional)|仓库（可选）|儲存庫（選填）|リポジトリ（任意）|저장소 (선택)|Repositori (pilihan)
trigger|Trigger|触发方式|觸發方式|トリガー|트리거|Pencetus
manual|Manual|手动|手動|手動|수동|Manual
push|Push events / detected changes|提交事件／检测到变更|提交事件／偵測到變更|プッシュイベント／変更検出|푸시 이벤트 / 변경 감지|Acara push / perubahan dikesan
branch|Branch|分支|分支|ブランチ|브랜치|Cabang
tests_enabled|Automatic tests|自动测试|自動測試|自動テスト|자동 테스트|Ujian automatik
ai_enabled|AI audit|AI 审计|AI 審計|AI 監査|AI 감사|Audit AI
test_command|Test command|测试命令|測試命令|テストコマンド|테스트 명령|Arahan ujian
junit_path|JUnit result path|JUnit 结果路径|JUnit 結果路徑|JUnit 結果パス|JUnit 결과 경로|Laluan keputusan JUnit
runner_kind|Runner placement|Runner 执行位置|Runner 執行位置|Runner 配置|Runner 배치|Penempatan Runner
pool|Shared Runner pool|共享 Runner 池|共用 Runner 池|共有 Runner プール|공유 Runner 풀|Kumpulan Runner berkongsi
host|Specific Runner host|指定 Runner 主机|指定 Runner 主機|指定 Runner ホスト|지정 Runner 호스트|Hos Runner tertentu
host-group|Runner host group|Runner 主机组|Runner 主機群組|Runner ホストグループ|Runner 호스트 그룹|Kumpulan hos Runner
runner_ref|Runner host / group|Runner 主机／分组|Runner 主機／群組|Runner ホスト／グループ|Runner 호스트 / 그룹|Hos / kumpulan Runner
runner_labels|Runner labels (one per line)|Runner 标签（每行一个）|Runner 標籤（每行一個）|Runner ラベル（1行に1件）|Runner 레이블 (줄당 하나)|Label Runner (satu setiap baris)
ai_endpoint|OpenAI-compatible Chat Completions endpoint|兼容 OpenAI 的 Chat Completions 地址|相容 OpenAI 的 Chat Completions 位址|OpenAI 互換 Chat Completions URL|OpenAI 호환 Chat Completions 주소|Alamat Chat Completions serasi OpenAI
ai_model|Model|模型|模型|モデル|모델|Model
ai_credential_id|Encrypted AI credential|加密 AI 凭据|加密 AI 憑證|暗号化 AI 認証情報|암호화 AI 자격 증명|Kelayakan AI disulitkan
ai_timeout_seconds|Model timeout (seconds)|模型超时（秒）|模型逾時（秒）|モデルのタイムアウト（秒）|모델 시간 제한 (초)|Had masa model (saat)
ai_max_tokens|Token budget|Token 预算|Token 預算|トークン予算|토큰 예산|Bajet token
max_files|Maximum scanned files|最大扫描文件数|最大掃描檔案數|最大スキャンファイル数|최대 스캔 파일 수|Maksimum fail diimbas
max_bytes|Maximum scanned bytes|最大扫描字节数|最大掃描位元組數|最大スキャンバイト数|최대 스캔 바이트 수|Maksimum bait diimbas
include|Included paths (one per line)|包含路径（每行一个）|包含路徑（每行一個）|対象パス（1行に1件）|포함 경로 (줄당 하나)|Laluan termasuk (satu setiap baris)
exclude|Excluded paths (one per line)|排除路径（每行一个）|排除路徑（每行一個）|除外パス（1行に1件）|제외 경로 (줄당 하나)|Laluan dikecualikan (satu setiap baris)
run|Run review|执行走查|執行審查|レビューを実行|검토 실행|Jalankan semakan
base_sha|Base SHA|基线 SHA|基準 SHA|ベース SHA|기준 SHA|SHA asas
head_sha|Target SHA|目标 SHA|目標 SHA|対象 SHA|대상 SHA|SHA sasaran
full_scan|Full repository scan|全仓扫描|全庫掃描|リポジトリ全体をスキャン|전체 저장소 스캔|Imbas seluruh repositori
pipeline|Dedicated pipeline|专用流水线|專用流水線|専用パイプライン|전용 파이프라인|Saluran paip khusus
pipelineHelp|A dedicated pipeline is generated from the review template and can be edited after saving.|保存后从走查模板生成专用流水线，可进入编辑。|儲存後從審查範本產生專用流水線，可進入編輯。|保存後にレビュー用テンプレートから専用パイプラインを生成し、編集できます。|저장 후 검토 템플릿에서 전용 파이프라인이 생성되며 편집할 수 있습니다.|Saluran paip khusus dijana daripada templat semakan selepas disimpan dan boleh disunting.
logs|Pipeline run and logs|流水线运行与日志|流水線執行與日誌|パイプライン実行とログ|파이프라인 실행 및 로그|Pelaksanaan dan log saluran paip
snapshot|Immutable configuration snapshot|不可变配置快照|不可變設定快照|不変の設定スナップショット|변경 불가 설정 스냅샷|Petikan konfigurasi kekal
severity|Severity|严重程度|嚴重程度|重大度|심각도|Keterukan
file|File / line|文件／行号|檔案／行號|ファイル／行|파일 / 줄|Fail / baris
message|Description|说明|說明|説明|설명|Penerangan
suggestion|Suggestion|建议|建議|提案|제안|Cadangan
open|Open|待处理|待處理|未対応|미해결|Terbuka
fixed|Fixed|已修复|已修復|修正済み|수정됨|Dibaiki
ignored|Ignored|已忽略|已忽略|無視|무시됨|Diabaikan
false_positive|False positive|误报|誤報|誤検知|오탐|Positif palsu
queued|Queued|排队中|排隊中|待機中|대기열|Dalam giliran
running|Running|运行中|執行中|実行中|실행 중|Sedang berjalan
failed|Task failed|任务失败|任務失敗|タスク失敗|작업 실패|Tugas gagal
passed|Passed|通过|通過|合格|통과|Lulus
skipped|Skipped|已跳过|已略過|スキップ|건너뜀|Dilangkau
not_enabled|Not enabled|未启用|未啟用|無効|비활성|Tidak diaktifkan
issues|AI completed: findings|AI 完成：发现问题|AI 完成：發現問題|AI 完了：検出あり|AI 완료: 문제 발견|AI selesai: penemuan
clean|AI completed: no findings|AI 完成：未发现问题|AI 完成：未發現問題|AI 完了：検出なし|AI 완료: 문제 없음|AI selesai: tiada penemuan
error|Error / waiting reason|错误／等待原因|錯誤／等待原因|エラー／待機理由|오류 / 대기 사유|Ralat / sebab menunggu
scanned|Scanned files|已扫描文件|已掃描檔案|スキャン済みファイル|스캔된 파일|Fail diimbas
required|Select at least one subject and period.|请选择至少一个对象与周期。|請選擇至少一個對象與週期。|対象と期間を1つ以上選択してください。|대상과 기간을 하나 이상 선택하세요.|Pilih sekurang-kurangnya satu subjek dan tempoh.`;
export default Object.fromEntries(
  locales.map((locale, index) => [
    locale,
    Object.fromEntries(
      rows.split("\n").map((row) => {
        const [key, ...values] = row.split("|");
        return [key!, values[index]!];
      }),
    ),
  ]),
);
