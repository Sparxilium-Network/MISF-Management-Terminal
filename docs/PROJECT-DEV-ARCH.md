# 原始碼檔案說明

> 最後修改：2026-11-08 ｜ 基於 commit：`10ff78c`

每個 `.rs` 檔案負責什麼。整體設計與資料流見 [架構](architecture.md)，建置與慣例見 [開發說明](PROJECT-DEV.md)。新增、改名或刪除 `.rs` 檔案時，請一併更新本文件。

Workspace 有三個 crate：`core`（`misf-core`，核心函式庫與 JSON CLI）、`app`（`misf`，桌面 GUI）、`third_party/LUKSbox/luksbox-fido2`（LUKSbox FIDO2 的本機相容性副本）。依賴方向是 `app` → `core` → `luksbox-fido2`；`core` 不知道 UI 存在。

## core：核心

### 入口與請求

| 檔案 | 作用 |
| --- | --- |
| `core/src/lib.rs` | crate 根：宣告模組、禁止 `unsafe`（只有 Windows WebAuthn 例外）。`dispatch` 處理已解鎖後的主機請求：`list`、`save`、`delete`、`convert_storage`、金鑰與分類、`arrange`、`probe`、`trust`、`test`、`check`／`upgrade`／`dist_upgrade`、`batch`（交給 `batch.rs`）、`reboot`（交給 `reboot.rs`），並負責儲存時沿用伺服器端欄位。`dispatch_with` 另外接收批次觀察者。 |
| `core/src/reboot.rs` | 重新開機：固定命令 `shutdown -r +1`（root 直接、一般帳號經 sudo），只限已偵測為 APT 的主機；回傳主機的回覆。 |
| `core/src/batch.rs` | 批次作業：ID 去重與上限、拒絕混合套件管理器、最多 8 台並行；每台完成即儲存並呼叫 `Observer::finished`，失敗時呼叫 `Observer::failed` 並在得到 `Decision`（繼續／中斷）前不開始新主機；回應含 `results` 與未執行的 `skipped`。`Unattended` 給 CLI 與測試用。 |
| `core/src/main.rs` | JSON CLI 執行檔：從 stdin 讀一個請求或請求陣列（上限 8 MiB，zeroize 緩衝），交給 `Session`，以 `progress`／`result`／`error` 行輸出 JSON。 |
| `core/src/session.rs` | GUI 與 CLI 共用的入口 `Session`：每個請求先寫稽核開始紀錄、套用 debug 限制（只允許 loopback SSH、禁止真實 Passkey、備份限測試目錄），再交給 `StorageSession`，最後記錄結果。 |
| `core/src/runtime.rs` | `RuntimePaths`：解析 `--debug.dir`／`--debug.allow-passkey` 與 `MISF_DATA_DIR`，決定資料目錄。debug 設定檔不解析、不遷移正式資料目錄，失敗不回退。 |
| `core/src/audit.rs` | 操作日誌：只接受固定的 `Action`／`Outcome` 列舉、主機顯示名稱與槽號，以檔案鎖追加到 `logs/actions-YYYY-MM-DD.jsonl`（UTC）並同步磁碟。 |

### 資料模型與儲存

| 檔案 | 作用 |
| --- | --- |
| `core/src/model.rs` | 資料結構 `Host`、`SshKey`、`HostGroup`、`Package`、`CheckResult`（`is_upgrade`）、`UpgradePlan`、`RestartStatus`、`ConnectionTest`；主機欄位驗證、`redact`（遮蔽錯誤與輸出中的密碼／位址）、`manager()`（選用或偵測到的套件管理器）、`decode_private_key`（拒絕 RSA）。 |
| `core/src/store.rs` | `Database`（記憶體中的完整主機資料）與明文 `Store`：只在建立加密主機庫時讀取舊版 `hosts.db`／`hosts.json`，含跨程序檔案鎖與舊 JSON 解析。 |
| `core/src/sqlite.rs` | SQLite 讀寫：開檔前拒絕 WAL、強化連線設定、`quick_check`、`application_id`／`user_version` 與結構逐列比對；`load`／`save` 在 `Database` 與資料表之間轉換；記憶體映像的序列化與 `sqlite3_deserialize`。 |
| `core/src/vault.rs` | `StorageSession`：LUKSbox 容器的建立（含重新解鎖驗證）、密碼／Passkey 解鎖、鎖定、結構升級詢問、金鑰槽新增／命名／撤銷、加密備份、舊版明文轉入與刪除。處理 `storage_status` 與所有 `vault_*` 請求，解鎖後把主機請求轉給 `dispatch`。 |
| `core/src/db_upgrade/mod.rs` | 遷移清單 `MIGRATIONS`：`MIGRATIONS[n]` 把 vN 升到 vN+1，只能追加。 |
| `core/src/db_upgrade/v0_v1.rs` | v1：`hosts`、`check_results`、`packages`。 |
| `core/src/db_upgrade/v1_v2.rs` | v2：`hosts.manager_choice` 與 `key_slots`（金鑰槽名稱）。 |
| `core/src/db_upgrade/v2_v3.rs` | v3：`ssh_keys` 與 `hosts.key_id`。 |
| `core/src/db_upgrade/v3_v4.rs` | v4：`host_groups` 與 `hosts.group_id`。 |
| `core/src/db_upgrade/v4_v5.rs` | v5：`connection_tests`（每台主機最後一次測試連線）。 |
| `core/src/db_upgrade/v5_v6.rs` | v6：`restart_status` 與 `restart_items`（重啟提示）。 |

### 解鎖用的 Passkey

| 檔案 | 作用 |
| --- | --- |
| `core/src/passkey.rs` | 選擇原生驗證器（Windows WebAuthn 或 Linux libfido2），註冊時兩次讀 PRF 確認一致，並以 `Fido2HmacSecret` 槽開啟容器。 |
| `core/src/windows_passkey.rs` | Windows `webauthn.dll` 的 FFI 轉接層（全專案唯一允許 `unsafe` 的檔案）：建立 client JSON 與隨機 challenge、依 API 版本選 options、啟用 PRF 或 hmac-secret、以 RAII 釋放原生輸出，區分參數錯誤與使用者取消。 |

### SSH 與套件管理

| 檔案 | 作用 |
| --- | --- |
| `core/src/ssh.rs` | SSH 傳輸：`probe`（只取主機指紋，不送帳密）、`connect`（核對指紋）、`authenticate`（密碼或私鑰）、`execute`（執行一條命令、偵測 sudo 提示並最多回答一次、2 MiB 輸出上限）、`test_connection`（產生 `ConnectionTest`）。不含任何套件命令。 |
| `core/src/transcript.rs` | 把遠端輸出轉成可讀紀錄：統一換行、移除 ANSI 與終端控制字串；`heading` 產生依顯示寬度對齊的星號框標題，供執行輸出與 LOG 分段。 |
| `core/src/package_manager/mod.rs` | `Mode`（check／upgrade／dist_upgrade）、`Manager` 列舉與 `Commands { update, list, upgrade, dist_upgrade, simulate, status }`，依管理器分派到各檔案；`manager_commands`／`package_commands` 列出確認視窗要顯示的命令；`SUDO_PROMPT` 標記。 |
| `core/src/package_manager/detect.rs` | `DETECT` 偵測命令與 `parse_platform`：核心、發行版顯示名稱、可用的套件管理器與優先順序。 |
| `core/src/package_manager/operation.rs` | 一台主機的檢查／升級／dist-upgrade 流程：偵測 → 選定管理器（不得與紀錄或確認內容不同）→ 更新索引 → 讀清單 →（升級或 dist-upgrade → 再讀清單）→（有 kept back：唯讀模擬 dist-upgrade）→（APT：讀重啟狀態），選用步驟失敗不影響結果；各段輸出以星號框標題分隔，最後組成 `CheckResult`。 |
| `core/src/package_manager/apt.rs` | APT 的固定命令（含 `DIST_UPGRADE`、唯讀的 `SIMULATE`，以及 root 直接執行／一般帳號整段放進 sudo 的 `STATUS`／`SUDO_STATUS`）、索引更新失敗判斷、`apt list --upgradable` 解析、phasing 延後與 kept back 套件解析、`parse_plan`（`apt-get -s` 的 `Inst`／`Remv`），以及 `parse_restart`。 |
| `core/src/package_manager/dnf.rs` | DNF 的固定命令。 |
| `core/src/package_manager/yum.rs` | YUM 的固定命令。 |
| `core/src/package_manager/rpm.rs` | DNF／YUM 共用：索引更新失敗判斷、`check-update` 退出碼（0／100）與清單解析（含 yum 換行、rpm 已安裝版本）。 |
| `core/src/package_manager/pkg.rs` | FreeBSD／DragonFly pkg 的固定命令、失敗判斷與 `pkg version` 解析。 |

### 整合測試

| 檔案 | 作用 |
| --- | --- |
| `core/tests/workflow.rs` | 在 loopback 啟動測試 SSH 伺服器，端到端驗證密碼／私鑰登入、指紋、PTY 與 sudo、APT／DNF 檢查與升級、失敗處理、批次、主機分類、共用金鑰，以及透過 LUKSbox 的儲存與 CLI 相容性。 |

## app：桌面 GUI

| 檔案 | 作用 |
| --- | --- |
| `app/build.rs` | Windows 建置時嵌入程式圖示與版本資訊資源。 |
| `app/src/main.rs` | GUI 執行檔入口：處理 `--help`、解析 `RuntimePaths`、開啟稽核日誌、設定視窗與圖示後啟動 eframe；release 版不開主控台視窗。 |
| `app/src/ui.rs` | 主視窗 `ManagementApp`：側欄主機清單（分類、拖曳、收合、搜尋）、主機編輯、結果卡片（套件、已升級、重啟提示、執行輸出、連線資訊分頁）、批次作業表格（逐台更新、雙擊跳到主機頁）與批次失敗詢問、主機頁「進階操作」選單（升級、dist-upgrade、重新開機；開關狀態自行保存，讓項目的右鍵複製選單不會關掉它）、重新開機的兩次確認、「需要 dist-upgrade」報告、按鈕右鍵複製命令、升級確認、解鎖與金鑰管理、設定與關於、明文副本與結構升級詢問、設定中的 LOG 頁與匯出。只送出請求並顯示結果，不直接連線或讀寫檔案。 |
| `app/src/session_log.rs` | 只存在記憶體的完整工作階段 LOG：操作標題（`banner`）、完整請求與回應（`mask` 只遮蔽密碼、私鑰、密語、PIN 欄位）、進度、錯誤，以及每台主機的區塊（`host`：ID、帳號@位址、指紋、套件、重啟狀態、遠端原始輸出，以 `END OF` 結束）；超過 16 MiB 捨棄最舊的；`export` 寫成文字檔。以 `Zeroizing` 保存，關閉程式即消失。 |
| `app/src/worker.rs` | 背景執行緒與 Tokio runtime：從佇列取出請求交給 `Session`，以 `Event` 回傳進度與結果並喚醒畫面；`Relay` 是批次觀察者，轉送每台結果與失敗詢問，並等待 UI 經 `decisions` 送回的決定。 |
| `app/src/preferences.rs` | 未加密的 `ui-settings.json`：語言、外觀、視窗大小、主機清單顯示、批次排序／欄位／明細位置、預設解鎖方式；以暫存檔原子替換寫入。 |
| `app/src/i18n.rs` | 語系：內嵌 `app/locales/*.json`，以 key 與 `{name}` 參數取字，缺字時回退繁體中文；測試檢查 UI 用到的 key 在兩種語言都存在且參數一致。 |
| `app/src/theme.rs` | 淺色／深色／跟隨系統的配色 `Palette` 與 egui 樣式。 |

## third_party/LUKSbox/luksbox-fido2：FIDO2 相容性副本

上游 LUKSbox v0.6.1 的 FIDO2 crate，只為更新依賴而做了有紀錄的本機修改（見 [`PATCHES.md`](../third_party/LUKSbox/PATCHES.md)）。除非上游更新，否則不在這裡加功能。

| 檔案 | 作用 |
| --- | --- |
| `build.rs` | 啟用 `hardware` 功能時尋找並連結 libfido2，以 bindgen 產生綁定。 |
| `src/lib.rs` | crate 根與對外匯出。 |
| `src/authenticator.rs` | `Fido2Authenticator` trait、RP ID 與 WebAuthn PRF → hmac-secret salt 的換算。 |
| `src/error.rs` | 錯誤型別。 |
| `src/protocol.rs` | CTAP2 hmac-secret 擴充的密碼運算（`pinUvAuthProtocol = 1`）。 |
| `src/ffi.rs` | libfido2 FFI 綁定。 |
| `src/hid.rs` | 以 libfido2 操作實體 USB 安全金鑰（Linux）。 |
| `src/webauthn.rs` | Windows WebAuthn 驗證器（MISF 實際使用的是 `core/src/windows_passkey.rs`）。 |
| `src/webauthn_paths.rs` | Windows WebAuthn 裝置路徑分類。 |
| `src/mock.rs` | 測試用的記憶體內模擬驗證器。 |
