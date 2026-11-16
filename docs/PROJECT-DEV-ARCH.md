# 原始碼檔案說明

> 最後修改：2026-11-17 ｜ 基於 commit：`8aaa1b9`

每個 `.rs` 檔案負責什麼。整體設計與資料流見 [架構](architecture.md)，建置與慣例見 [開發說明](PROJECT-DEV.md)。新增、改名或刪除 `.rs` 檔案時，請一併更新本文件。

Workspace 有四個 crate：`core`（`misf-core`，核心函式庫與 JSON CLI）、`app`（`misf`，桌面 GUI）、`sshtest`（`misf-sshtest`，只有測試，不編進 app）、`third_party/LUKSbox/luksbox-fido2`（LUKSbox FIDO2 的本機相容性副本）。依賴方向是 `app` → `core` → `luksbox-fido2`，`sshtest` → `core`；`core` 不知道 UI 存在。

## core：核心

### 入口與請求

| 檔案 | 作用 |
| --- | --- |
| `core/src/lib.rs` | crate 根：宣告模組、禁止 `unsafe`（只有 Windows WebAuthn 例外）。`dispatch` 處理已解鎖後的主機請求：`list`、`save`、`delete`、`convert_storage`、金鑰與分類、`arrange`、`probe`、`trust`、`test`、`check`／`upgrade`／`dist_upgrade`、`batch`（交給 `batch.rs`）、`reboot`（交給 `reboot.rs`），並負責儲存時沿用伺服器端欄位。`dispatch_with` 另外接收批次觀察者。 |
| `core/src/reboot.rs` | 重新開機：固定命令 `shutdown -r +1`（root 直接、一般帳號經 sudo），只限已偵測為 APT 的主機；回傳主機的回覆。 |
| `core/src/batch.rs` | 批次作業：ID 去重與上限、拒絕混合套件管理器、最多 8 台並行；每台完成即儲存並呼叫 `Observer::finished`，失敗時呼叫 `Observer::failed` 並在得到 `Decision`（繼續／中斷）前不開始新主機；收到 `Cancel` 時中止執行中的工作，已完成的照常儲存，被中斷的升級以 `stopped_upgrade` 存為結果未知；回應含 `results`、未執行的 `skipped` 與執行中被中斷的 `cancelled`。`Unattended` 給 CLI 與測試用。 |
| `core/src/cancel.rs` | `Cancel`：使用者中斷的訊號（`request`／`reset`／`requested`），`CANCELLABLE` 列出可中斷的請求。 |
| `core/src/main.rs` | JSON CLI 執行檔：從 stdin 讀一個請求或請求陣列（上限 8 MiB，zeroize 緩衝），交給 `Session`，以 `progress`／`result`／`error` 行輸出 JSON。 |
| `core/src/session.rs` | GUI 與 CLI 共用的入口 `Session`：每個請求先寫稽核開始紀錄、套用 debug 限制（只允許 loopback SSH、禁止真實 Passkey、備份限測試目錄），再交給 `StorageSession`，最後記錄結果。`set_cancel` 接收中斷訊號：單台的檢查、升級、dist-upgrade 與測試連線在這裡以 `select!` 中斷（`stopped` 產生回應，升級存為結果未知），批次交給 `batch::run`；中斷記為 `cancelled`。 |
| `core/src/runtime.rs` | `RuntimePaths`：解析 `--debug.dir`／`--debug.allow-passkey` 與 `MISF_DATA_DIR`，決定資料目錄。debug 設定檔不解析、不遷移正式資料目錄，失敗不回退。 |
| `core/src/audit.rs` | 操作日誌：只接受固定的 `Action`／`Outcome` 列舉、主機顯示名稱與槽號，以檔案鎖追加到 `logs/actions-YYYY-MM-DD.jsonl`（UTC）並同步磁碟。 |

### 資料模型與儲存

| 檔案 | 作用 |
| --- | --- |
| `core/src/model.rs` | 資料結構 `Host`、`SshKey`、`HostGroup`、`Package`、`CheckResult`（`is_upgrade`）、`UpgradePlan`、`RestartStatus`、`ConnectionTest`；主機欄位驗證、`redact`（遮蔽錯誤與輸出中的密碼／位址）、`manager()`（選用或偵測到的套件管理器）、`MANAGER_CHOICES`（可手動指定的套件管理器）、`forget_unsupported`（讀取時把舊版的 BSD／YUM／pkg 設定改回 Linux 與自動偵測）、`decode_private_key`（拒絕 RSA）。 |
| `core/src/store.rs` | `Database`（記憶體中的完整主機資料）與明文 `Store`：只在建立加密主機庫時讀取舊版 `hosts.db`／`hosts.json`，含跨程序檔案鎖與舊 JSON 解析。 |
| `core/src/sqlite.rs` | SQLite 讀寫：開檔前拒絕 WAL、強化連線設定、`quick_check`、`application_id`／`user_version` 與結構逐列比對；`load`／`save` 在 `Database` 與資料表之間轉換；記憶體映像的序列化與 `sqlite3_deserialize`。 |
| `core/src/vault.rs` | `StorageSession`：LUKSbox 容器的建立（含重新解鎖驗證）、密碼／Passkey 解鎖、鎖定、結構升級詢問、金鑰槽新增／命名／撤銷、加密備份、舊版明文轉入與刪除（`hosts.db`、`hosts.json`、`hosts.v<N>.bak.db` 依序逐一確認）；`hosts_for_audit` 以一次讀取取得批次稽核用的主機。處理 `storage_status` 與所有 `vault_*` 請求，解鎖後把主機請求轉給 `dispatch`。 |
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
| `core/src/ssh.rs` | SSH 傳輸：`probe`（只取主機指紋，不送帳密）、`connect`（核對指紋）、`authenticate`（密碼或私鑰）、`execute`（執行一條命令、偵測 sudo 提示並最多回答一次——`count_prompts` 只掃描新資料加上前一段結尾不足一個提示長度的位元組，不重複掃描整個緩衝區、32 MiB 輸出上限；同時經 `Echo` 把命令與逐行輸出即時送給終端機）、`test_connection`（產生 `ConnectionTest`）。不含任何套件命令。 |
| `core/src/shell.rs` | 互動 shell：`open` 登入主機、開 `xterm-256color` PTY 與 login shell，等到 PTY 與 shell 兩個回覆後才返回，再以 `tokio::spawn` 讓 `pump` 在背景雙向搬運位元組（輸出交給 `Screen`，結束時呼叫 `Screen::ended`）。`Shells` 是 worker 與 UI 共用的開啟中 shell 表（編號只增不重用），每個 shell 記錄尚未畫出的輸出量，超過 `WINDOW`（1 MiB）時 `pump` 停止讀取 channel，UI 以 `Shells::drawn` 回報後再繼續；寫入時遠端不讀，仍可被關閉要求中斷；`Input` 為按鍵位元組、視窗大小或關閉。輸入與輸出不經請求佇列，也不寫入稽核紀錄或工作階段記錄。`StorageSession` 的 `shell_open` 請求開啟 shell，`vault_lock` 先 `close_all`；debug 模式與其他遠端操作一樣只允許 loopback。 |
| `core/src/console.rs` | 即時遠端輸出：`Console` trait（`command`／`output`，以主機 ID 區分）、不顯示的 `Silent`，以及每台主機一份的 `Echo`（帶著該主機的密碼、sudo 密碼、私鑰密語與私鑰各行，逐行遮蔽）。`Lines` 收到行尾（`\n`，或進度列用的 `\r`；跨兩段的 `\r\n` 只算一次）才送出整行，所以 ANSI 序列、UTF-8 字元與密碼不會被切開，且只掃描新進來的資料。單行超過 `LINE_LIMIT`（8 KiB）時只保留「上限＋最長密碼長度」的原始位元組，之後到行尾的資料丟棄；送出前先清理、遮蔽，再固定丟掉結尾「最長密碼長度」個字元（清理可能讓文字變短，這一步保證被切斷的半段密碼不會出現），超過上限再切到上限，並加上「（此行過長，已截斷）」。存檔的完整輸出不受影響；清理方式與 `transcript::readable_output` 相同，並移除 sudo 提示標記。`StorageSession`／`Session::set_console` 設定接收端，`dispatch_with` 傳給單台檢查／升級／dist-upgrade／重新開機與批次的每台主機；測試連線不顯示。 |
| `core/src/transcript.rs` | 把遠端輸出轉成可讀紀錄：統一換行、移除 ANSI 與終端控制字串；`heading` 產生依顯示寬度對齊的星號框標題，供執行輸出與 LOG 分段。 |
| `core/src/package_manager/mod.rs` | `Mode`（check／upgrade／dist_upgrade）、`Manager` 列舉與 `Commands { update, list, upgrade, dist_upgrade, simulate, preview, status }`，依管理器分派到各檔案；`manager_commands`／`package_commands` 列出確認視窗要顯示的命令；`SUDO_PROMPT` 標記。 |
| `core/src/package_manager/detect.rs` | `DETECT` 偵測命令與 `parse_platform`：核心、發行版顯示名稱、可用的套件管理器與優先順序。 |
| `core/src/package_manager/operation.rs` | 一台主機的檢查／升級／dist-upgrade 流程：偵測 → 選定管理器（不得與紀錄或確認內容不同）→ 更新索引 → 讀清單 →（檢查且有待更新：唯讀模擬 upgrade，找出 phasing 延後的套件）→（升級或 dist-upgrade → 再讀清單）→（有 kept back：唯讀模擬 dist-upgrade）→（APT：讀重啟狀態），選用步驟失敗不影響結果；各段輸出以星號框標題分隔，最後組成 `CheckResult`。 |
| `core/src/package_manager/apt.rs` | APT 的固定命令（含 `DIST_UPGRADE`、唯讀的 `SIMULATE` 與檢查時用的 `PREVIEW`，以及 root 直接執行／一般帳號整段放進 sudo 的 `STATUS`／`SUDO_STATUS`）、索引更新失敗判斷、`apt list --upgradable` 解析、phasing 延後與 kept back 套件解析、`parse_plan`（`apt-get -s` 的 `Inst`／`Remv`），以及 `parse_restart`。 |
| `core/src/package_manager/dnf.rs` | DNF 的固定命令。 |
| `core/src/package_manager/rpm.rs` | DNF 的索引更新失敗判斷、`check-update` 退出碼（0／100）與清單解析（含換行、rpm 已安裝版本）。 |
| `core/src/package_manager/pacman.rs` | pacman 的固定命令（`checkupdates` 檢查、`pacman -Syu` 升級）、失敗判斷、退出碼（0／2）與 `name old -> new` 解析。 |
| `core/src/package_manager/apk.rs` | apk 的固定命令、索引警告與 `ERROR:` 的失敗判斷，以及 `apk list --upgradable` 解析。 |

### 整合測試

| 檔案 | 作用 |
| --- | --- |
| `core/tests/workflow.rs` | 在 loopback 啟動測試 SSH 伺服器，端到端驗證密碼／私鑰登入、指紋、PTY 與 sudo、APT／DNF 檢查與升級、失敗處理、批次、主機分類、共用金鑰，以及透過 LUKSbox 的儲存與 CLI 相容性。 |
| `sshtest/src/lib.rs` | 套件管理器故障測試的共用框架：`System`（一個套件管理器的測試主機，映像在 `sshtest/<管理器>/docker/`）、`Container`（以 `docker` 啟動測試容器、`docker exec`）、`Relay`（MISF 與容器之間的 TCP 中繼，可斷線、凍結、恢復）、`Lab`（像 app 一樣儲存主機、信任指紋、測試連線，再以 `dispatch_with` 執行並收集即時輸出），以及各管理器通用的斷言。每個套件管理器的測試放在自己的資料夾，不放進這裡。 |
| `sshtest/apt/main.rs` | 測試目標 `apt` 的入口（`#[ignore]`，需要 Docker），對真實 Debian／Ubuntu 容器。斷言記錄目前行為，缺口列在 `sshtest/apt/README.md`。 |
| `sshtest/apt/support.rs` | APT 測試共用：啟動 APT 主機、`Dpkg`（dpkg 狀態、`dpkg --audit`、apt 是否仍在執行）、共用斷言。 |
| `sshtest/apt/upgrade.rs` | 正常升級與 reboot-required、debconf 與 conffile 不詢問、kept back 與 dist-upgrade。 |
| `sshtest/apt/failures.rs` | postinst 失敗、套件庫索引或套件檔抓不到、dpkg 被殺、鎖被佔用。 |
| `sshtest/apt/output.rs` | 不正常的輸出、`E:` 開頭的腳本輸出、輸出中的 sudo 提示標記。 |
| `sshtest/apt/prompts.rs` | 對話框在 `TERM=dumb` 下失敗、畫出來的對話框、等待輸入的腳本。 |
| `sshtest/apt/connection.rs` | 升級途中斷線與網路無聲中斷。 |
| `sshtest/dnf/main.rs` | 測試目標 `dnf` 的入口（`#[ignore]`，需要 Docker），對真實 Rocky Linux／AlmaLinux 9 容器。斷言記錄目前行為，缺口列在 `sshtest/dnf/README.md`。 |
| `sshtest/dnf/support.rs` | DNF 測試共用：啟動 DNF 主機、`Rpm`（已安裝的版本、dnf 是否仍在執行）。 |
| `sshtest/dnf/upgrade.rs` | 正常升級與連帶安裝相依、設定檔保留、root 不經 sudo、沒有 dist-upgrade。 |
| `sshtest/dnf/failures.rs` | `%post`／`%pre` 失敗、相依不滿足、套件庫或套件檔抓不到、dnf 被殺、等待鎖。 |
| `sshtest/dnf/output.rs` | 不正常的輸出、`Error:` 開頭的腳本輸出、輸出中的 sudo 提示標記。 |
| `sshtest/dnf/prompts.rs` | 對話框在 `TERM=dumb` 下失敗、標準輸入上的問題、終端機上的對話框與問題。 |
| `sshtest/dnf/connection.rs` | 升級途中斷線（root、sudo 加 PTY）與網路無聲中斷。 |
| `sshtest/pacman/main.rs` | 測試目標 `pacman` 的入口（`#[ignore]`，需要 Docker），對真實 Arch Linux 容器。斷言記錄目前行為，缺口與該管理器的特殊行為列在 `sshtest/pacman/README.md`。 |
| `sshtest/pacman/support.rs` | pacman 測試共用：啟動主機、`Pacman`（已安裝的版本、pacman 是否仍在執行、鎖檔是否存在）。 |
| `sshtest/pacman/upgrade.rs` | 檢查不經 sudo 也不動系統資料庫、升級與連帶安裝相依、`.pacnew`、沒有 dist-upgrade。 |
| `sshtest/pacman/failures.rs` | 套件腳本失敗、相依不滿足、套件庫或套件檔抓不到、缺 checkupdates／fakeroot、pacman 被殺後留下鎖檔、鎖被佔用。 |
| `sshtest/pacman/output.rs` | 不正常的輸出、`error:` 開頭的腳本輸出、輸出中的 sudo 提示標記。 |
| `sshtest/pacman/prompts.rs` | 對話框在 `TERM=dumb` 下失敗、標準輸入上的問題、終端機上的對話框與問題。 |
| `sshtest/pacman/connection.rs` | 升級途中斷線（有無 PTY、是否經 sudo，含留下鎖檔的情況）與網路無聲中斷。 |
| `sshtest/apk/main.rs` | 測試目標 `apk` 的入口（`#[ignore]`，需要 Docker），對真實 Alpine 容器。斷言記錄目前行為，缺口與該管理器的特殊行為列在 `sshtest/apk/README.md`。 |
| `sshtest/apk/support.rs` | apk 測試共用：啟動主機、`Apk`（已安裝的版本、等 apk 結束）。 |
| `sshtest/apk/upgrade.rs` | 正常升級與連帶安裝相依、`.apk-new`、免密碼 sudo、沒有 dist-upgrade。 |
| `sshtest/apk/failures.rs` | 套件腳本失敗、相依不滿足、索引打不開或簽章不受信任、套件檔讀不到、apk 被殺、鎖被佔用。 |
| `sshtest/apk/output.rs` | 不正常的輸出、`ERROR:` 開頭的腳本輸出、輸出中的 sudo 提示標記、標準輸入上的問題。 |
| `sshtest/apk/prompts.rs` | 對話框開不起來、終端機上的對話框與問題。 |
| `sshtest/apk/connection.rs` | 升級途中斷線（有無 PTY）與網路無聲中斷。 |

## app：桌面 GUI

| 檔案 | 作用 |
| --- | --- |
| `app/build.rs` | 以 fluent 樣式編譯 `app/ui/main.slint`（debug 建置保留元件名稱供測試尋找）；Windows 建置時嵌入程式圖示與版本資訊資源；產生 `MISF_BUILD_NAME`（GitHub Actions 建置與 Release 同名，本機為 `v<Cargo 版本>`），供設定頁與 LOG 顯示版本。 |
| `app/src/main.rs` | GUI 執行檔入口：處理 `--help`、解析 `RuntimePaths`、開啟稽核日誌、再交給 `ui::run` 開啟 Slint 視窗；release 版不開主控台視窗。 |
| `app/src/ui/mod.rs` | 建立主視窗與設定視窗（`build`）、執行事件迴圈（`run`）、`Ctx::update` 修改狀態後以 `refresh` 把狀態推到兩個視窗；監看 winit 輸入事件供閒置自動鎖定、記錄視窗大小，並在右鍵按住時設定 `PopMenu.right-down`（`track_right_button`）；`sync_rows`／`keep_inner` 就地更新清單，避免拖曳或雙擊中的元件被重建；`capture` 功能下的截圖、`MISF_FIXTURE` 假資料與 `MISF_OPEN_MENU` 範例選單。 |
| `app/src/ui/app.rs` | 視窗狀態 `App`（原 `ManagementApp`）與對話框資料結構：送出請求、處理背景事件與回應（`received`）、稽核與工作階段記錄、清除主機快取、閒置自動鎖定（`auto_lock`，作業中不鎖）、關閉時儲存視窗大小（`shutdown`）。只送出請求並保存結果，不直接連線或讀寫主機資料。 |
| `app/src/ui/format.rs` | 由主機與結果產生的文字：套件管理器與系統標籤、狀態、重啟摘要、連線資訊列、可複製的命令（`copied_commands`），以及連線與重啟分頁的行資料 `Line`。 |
| `app/src/ui/terminal.rs` | 底部終端機面板（互動 shell 分頁接在操作分頁之後，`Terminal::shells`，`current` 跨兩者計算；`App::open_shell`／`shell_opened`／`shell_ended`，結束時以 `Action::ShellClose` 記稽核；`type_into_shell` 在忙碌或鎖定時拒絕輸入；視窗大小變更送給所有開啟中的 shell；模擬器解析與繪製以 `catch_unwind` 包住，出錯只讓該 shell 失敗；`clear_host_cache` 以 `end_shells` 結束 shell、記稽核並清空終端機；worker 崩潰時 `fail_shells`）：`Terminal` 保存最近一次檢查、升級或重新開機的每台主機分頁（`TabState`：等待、執行中、完成、失敗、略過）與各自的行（每台最多 `MAX_LINES` 行，超過時一次捨棄最舊的 2000 行）。`App::send` 依請求開新的一輪並把 `App::tool` 切到終端機；`App::process_events` 每輪最多處理 `EVENT_BUDGET` 個事件（shell 輸出依位元組計），剩下的由 `drain_events` 以計時器接續；`Event::Terminal` 加入行，`BatchHost`／`Done`／worker 崩潰收尾。`push` 對同一個 `VecModel` 只追加新行，換分頁或捨棄舊行時才重建；`wire` 也處理 `ToolWindow`（`App::toggle_tool`；共用高度存在偏好設定的 `tool_height`）。 |
| `app/src/ui/shell.rs` | 自繪標題列（`WindowChrome`：拖曳移動與邊緣縮放交給 winit 的 `drag_window`／`drag_resize_window`，關閉按鈕送出與系統關閉相同的 `CloseRequested`）、狀態列與其「過往操作」清單（`App::history`，經 `set_status`／`set_host_status`／`note_error` 記錄，進度訊息不記），主機／批次分頁切換；關閉主視窗時一併結束程式，操作進行中先以 `CloseDialog` 詢問。 |
| `app/src/ui/sidebar.rs` | 側欄主機清單：分類與收合、搜尋、卡片內容與高度、拖曳排序（`Layout` 計算放置位置，`arrange` 送出新順序）、分類名稱對話框。 |
| `app/src/ui/host_page.rs` | 主機頁與結果分頁：測試連線、指紋、檢查、「進階操作」選單（升級、dist-upgrade、重新開機，依主機能力停用）、升級提示、「需要 dist-upgrade」報告。 |
| `app/src/ui/batch.rs` | 批次作業：套件管理器選擇、可選主機、表格排序與分組標題、欄寬拖曳、上一台／下一台、雙擊或右鍵開主機頁、套件明細視窗。 |
| `app/src/ui/editor.rs` | 主機編輯對話框與 SSH 金鑰管理對話框；從檔案匯入私鑰在借用狀態之外開啟原生檔案視窗。 |
| `app/src/ui/vault.rs` | LUKSbox 狀態、解鎖／建立／金鑰槽管理對話框與密碼檢查、預設解鎖方式、明文副本與結構升級詢問、加密備份。 |
| `app/src/ui/dialogs.rs` | 升級確認、「需要 dist-upgrade」報告、重新開機兩次確認、批次失敗詢問、主機指紋、刪除主機、分類名稱與錯誤對話框。 |
| `app/src/ui/settings.rs` | 獨立的設定視窗：分類樹（`SettingsPage::PREFERENCES` 為「偏好設定」底下的外觀與主機清單／安全性／批次作業）、各偏好的套用與儲存、LOG 頁、關於頁（含主機庫的加密方式與檔案位置）。 |
| `app/src/ui/remote_shell.rs` | 互動 shell 分頁的終端機模擬：每個 `ShellTab` 一個 `vt100::Parser`（5000 行歷史）；`rows` 把畫面轉成每列的樣式片段（同樣式相連、寬字元單獨一段以對齊兩格、純空白不畫）；`key_bytes` 把 Slint 按鍵轉成終端機位元組（方向鍵依 application cursor 模式、F1–F12、Ctrl 組合、Alt 前綴）；`paste_bytes` 處理換行與 bracketed paste。 |
| `app/src/ui/logging.rs` | 工作階段記錄：請求標題、結果寫入與匯出。 |
| `app/src/ui/tests.rs` | 狀態邏輯測試，以及用 Slint 測試後端送出實際指標與鍵盤事件的畫面測試（進階選單、批次表格雙擊、欄寬拖曳、Esc 關閉明細、懸停提示時機與位置、側欄拖曳排序、右鍵複製命令、設定樹）。 |
| `app/ui/main.slint` | 主視窗 `AppWindow`（`no-frame`，標題列自繪）：標題列 `TopBar`（圖示、選單式的上層命令 `MenuItem`、視窗按鈕）、側欄、工作區、狀態列（`StatusBar`，只顯示最近一次結果；過往操作由 `ToolStripe` 開關），最左側的 `ToolStripe` 與側欄、工作區下方的工具視窗（`TerminalPanel` 或停靠的 `HistoryPanel`，依 `ToolWindow.open` 一次一個；側欄與工作區包在 `clip` 的 Rectangle 內，工具視窗開啟時它們被壓縮，而不是把狀態列推出視窗；主機頁與批次頁各包在 `ScrollView` 內，內容高度取可見高度與頁面最小高度的較大者，放不下時捲動而不是被裁切；高度只在拖曳與視窗縮放時夾住，不綁定視窗高度以免佈局迴圈），以及疊在上方的對話框、進階選單、右鍵選單 `MenuOverlay` 與提示框；`run-menu` 依選單行的 `action` 執行對應操作；匯出所有 global 給 Rust。 |
| `app/ui/theme.slint` | 配色 `Theme`、介面字型（Noto Sans TC／粗體／系統字型）、翻譯查詢 `T`（向 Rust 的語言目錄查字串）、外觀同步。 |
| `app/ui/widgets.slint` | 共用元件：懸停提示 `HintArea`（1.5 秒後顯示命令、效果與風險；右鍵開啟 `menu` 或複製命令）、自繪右鍵選單（`MenuLine`、`PopMenu`、`MenuArea`、`MenuOverlay`：外觀與進階選單一致，不用 Slint 內建的 `ContextMenuArea`，因其外觀無法套用 Theme；`MenuArea` 疊在控制項上方，只在 `right-down` 為真時啟用，因為控制項的 `TouchArea` 會抓住所有按下事件，停用的 `TouchArea` 才會讓事件穿過）、提示框、剪貼簿、彩色按鈕、三態勾選框、對話框外框等。 |
| `app/ui/title_bar.slint` | 自繪標題列的零件：`WindowChrome`（是否最大化與拖曳、縮放、最小化、最大化、關閉的回呼）、可拖曳且雙擊最大化的 `DragArea`、選單式文字命令 `MenuItem`、視窗按鈕 `CaptionButtons`、四邊四角的縮放把手與 1px 外框 `WindowEdges`（最大化時不顯示）。 |
| `app/ui/tool_window.slint` | IDE 式工具視窗：`ToolWindow`（目前開啟的工具視窗與共用高度）、視窗最左側的圖示直條 `ToolStripe`（終端機、過往操作）、可拖曳的上緣 `DockHandle` 與標題列 `DockTitle`（含收合鈕）。 |
| `app/ui/terminal.slint` | 終端機工具視窗 `TerminalPanel` 與 `TerminalState`：每台主機一個分頁（狀態圓點；shell 分頁另有 >_ 與關閉鈕）、「開啟 Shell」、「複製」；`ShellScreen` 以等寬字型量出格寬，依面板大小回報行列數，逐列畫出 `ShellRow`／`ShellRun` 與游標，`FocusScope` 接收按鍵（Ctrl+Shift+V 經隱藏的 `TextInput` 貼上），滾輪捲動歷史；以及固定行高的 `ListView`（不換行、過長省略）；捲到底時跟著新輸出捲動，往上捲就停止跟隨。 |
| `app/ui/sidebar.slint`、`host_page.slint`、`result_panel.slint`、`batch_page.slint`、`settings.slint`、`shell.slint` | 各頁畫面與其 global。 |
| `app/ui/dialogs/*.slint` | 主機編輯、SSH 金鑰、主機庫、確認類與簡短對話框。 |
| `app/src/session_log.rs` | 只存在記憶體的完整工作階段 LOG：操作標題（`banner`）、完整請求與回應（`mask` 只遮蔽密碼、私鑰、密語、PIN 欄位）、進度、錯誤，以及每台主機的區塊（`host`：ID、帳號@位址、指紋、套件、重啟狀態、遠端原始輸出，以 `END OF` 結束）；超過 16 MiB 捨棄最舊的；`export` 寫成文字檔。以 `Zeroizing` 保存，關閉程式即消失。 |
| `app/src/worker.rs` | 背景執行緒與 Tokio runtime：從佇列取出請求交給 `Session`，以 `Event` 回傳進度與結果，再以 `Wake` 喚醒 UI 執行緒；`Relay` 是批次觀察者，轉送每台結果與失敗詢問，並等待 UI 經 `decisions` 送回的決定；`Screen` 是終端機接收端，把命令與輸出轉成 `Event::Terminal`，互動 shell 的原始輸出轉成 `Event::ShellOutput`、結束轉成 `Event::ShellEnded`（`Worker::shells` 讓 UI 直接把按鍵送進 shell），大量輸出只在 `Worker::terminal` 旗標未設時喚醒一次 UI（UI 讀取前清除旗標）。請求 panic 時，若該請求是升級，先把尚未回報結果的主機（單台升級的那台，或批次中 `Relay` 尚未回報的主機）以 `batch::interrupted` 存入主機庫，再送出帶有這些主機與儲存結果的 `Event::Crashed`，然後結束執行緒；`Running` 只記錄請求的 action／operation／id／ids，不保留含密碼的原始請求。UI（`App::worker_crashed`）解除忙碌、把這些主機標示為結果未知，並在錯誤中逐行列出主機名稱，存檔失敗時另外說明重啟後會遺失標示；執行緒無聲結束時只解除忙碌並顯示錯誤。 |
| `app/src/preferences.rs` | 未加密的 `ui-settings.json`：語言、外觀、介面字型、視窗大小、主機清單顯示、批次排序／欄位／明細位置、預設解鎖方式、自動鎖定分鐘數（`AUTO_LOCK_MAX`、`AUTO_LOCK_DEFAULT`）；以暫存檔原子替換寫入。 |
| `app/src/i18n.rs` | 語系：內嵌 `app/locales/*.json`，以 key 與 `{name}` 參數取字，缺字時回退繁體中文；測試檢查 UI 用到的 key 在兩種語言都存在且參數一致。 |
| `app/src/theme.rs` | 外觀選項（跟隨系統／淺色／深色）與其儲存代碼。 |

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
