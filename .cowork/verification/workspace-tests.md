# 全工作區檢查結果

## 2026-11-10（右鍵複製命令修正，分支 `slint-rego`，基於 `3d90ce4`）

- 使用者回報進階選單內的右鍵複製失效。實際上 Slint 版所有 `HintArea` 的右鍵複製都失效：`ContextMenuArea` 包在控制項外層，但 Slint 的 `TouchArea` 對任何按鍵的按下都回傳 `GrabMouse`，事件分派在子項抓取後就停止（`i-slint-core` `input.rs` `send_mouse_event_to_item`），外層 `ContextMenu::input_event` 收不到右鍵。改為把 `ContextMenuArea` 疊在控制項上方：它只接受右鍵按下，其他事件回傳 `EventIgnored` 交給下方按鈕，左鍵與懸停提示不受影響。
- 新增測試：以實際右鍵事件點主機頁「測試連線」與進階選單「APT 升級…」，確認出現複製選單，以方向鍵＋Enter 選擇後剪貼簿內容等於提示的命令，且進階選單保持開啟。暫時還原修正時測試在「沒有複製選單」處失敗，確認能抓到此錯誤。測試後端的選單項目座標是彈出視窗內座標，因此以鍵盤選擇，不以點擊。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、20、57、24 項）。
- 未在執行中的 app 實際確認系統剪貼簿收到文字（`ClipboardSink` 以隱藏 `TextInput.copy()` 寫入，測試後端只能驗證 `Clip` 內容）。

## 2026-11-10（補齊 Skia 內含元件授權，分支 `slint-rego`，基於 `cc7ba2e`）

- 依 rust-skia 的建置參數與 Windows `skia.lib` 符號確認，預編譯 Skia 內含 libpng、libjpeg-turbo、zlib、Expat、Wuffs、D3D12 Memory Allocator；FreeType、HarfBuzz、ICU、libwebp 未編入。依 Skia `m153-0.101.2` 的 `DEPS` 釘選版本，自上游 googlesource 取得各授權全文放入 `licenses/Skia/third_party/`（Wuffs 無 NOTICE）；Linux 版另依 DEPS 附 Vulkan Memory Allocator 授權，但未實際建置 Linux 版驗證。
- `THIRD_PARTY_NOTICES.md` 加入元件表與 IJG 要求的致謝句；打包改為複製整個 `licenses/Skia/`。

## 2026-11-10（改用 Skia、批次欄寬拖曳修正，分支 `slint-rego`，基於 `2e87553`）

- 使用者比較後選定 Skia：繪圖引擎固定為 Skia，移除 femtovg 與 `skia` 建置選項。授權清單重新產生（含 skia-safe／skia-bindings），另附 Skia 本身的 BSD-3 全文 `licenses/Skia/LICENSE`，聲明檔列出預編譯檔內含的 libjpeg-turbo、libpng、zlib、Wuffs 等元件（各元件授權全文尚未逐一附上）。
- 使用者回報批次作業拉動不見：欄寬拖曳把手外的提示元件寬高為 0（先前把提示元件改為不伸展後暴露），補上尺寸；另外每次拖動都整個替換欄位清單，Slint 重建把手元件，拖動只生效一次且放開不存檔，改為就地更新。新增測試：以實際指標事件拖曳欄寬邊界，權重改變並在放開時存檔。
- 字型選項「系統字型」不再附「（原本的字型）」。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過。

## 2026-11-10（字型選項、Skia 比較版、提示位置修正，分支 `slint-rego`，基於 `62da7a9`）

- 使用者回報兩處跑版：主機頁「刪除」的提示區域把整列剩餘寬度吃掉（提示元件預設改為不橫向伸展），以及提示框在右下角超出視窗（覆蓋層沒有指定位置，被置中擺放而整體偏移約 100 px；改為鋪滿視窗並把提示框限制在視窗內）。新增測試：提示放在右下角時提示框仍完整在視窗內。
- 新增偏好設定「字型」：Noto Sans TC、Noto Sans TC 粗體、系統字型（原本的字型），兩個視窗同時套用；偏好設定往返與預設值有測試，三種選項截圖確認。
- 使用者回報字型霧霧的：femtovg 只做灰階反鋸齒、沒有字型微調。依使用者要求另建 Skia 版（`--features skia`）供並排比較，截圖上 Skia 的筆畫明顯較清楚。DPI 方面 winit 已設為每螢幕 DPI 感知，不是點陣放大造成。
- fmt、Clippy（`--all-features -D warnings`，含 Skia）通過；`cargo test --workspace` 全部通過（22、20、57、24 項）。

## 2026-11-10（Slint 版補完：Noto Sans TC、授權、Esc、互動測試，分支 `slint-rego`，基於 `a20ea4e`）

- 介面字型改為使用者提供的 Noto Sans TC 可變字重字型，內嵌於執行檔（約多 12 MB）；OFL 授權放在 `assets/fonts/OFL.txt`，打包時附為 `licenses/NotoSansTC/OFL.txt`。側欄卡片高度配合新字型的行高調整。
- 授權：Slint 依 GPL-3.0-only 使用，`deny.toml`／`about.toml` 加入 GPL-3.0-only，移除已不再使用的 OFL-1.1、Ubuntu-font-1.0 套件授權；刪除 `licenses/egui-fonts/`，重新產生 `THIRD_PARTY_LICENSES.html`；`THIRD_PARTY_NOTICES.md` 改寫 egui 段落為 Slint 與 Noto Sans TC。打包時先清空輸出的 `licenses/`，避免舊的 egui 字型授權殘留。
- 批次套件明細視窗與「需要 dist-upgrade」報告可按 Esc 關閉（與 egui 版相同，其他對話框不受 Esc 影響）。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、20、57、24 項）。Slint 測試後端以實際指標與鍵盤事件驗證：側欄拖曳排序送出新順序、懸停 1 秒不出現而 1.7 秒出現提示且移開即消失、Esc 關閉明細視窗、批次表格雙擊、進階選單、設定樹。
- 截圖確認新字型的主機頁、批次頁、緊湊側欄與設定視窗。仍未在執行中的 app 實際等待閒置自動鎖定（winit 輸入監看與每秒檢查無法在測試後端執行；計時邏輯有單元測試），右鍵複製只驗證了複製內容，未實際開啟右鍵選單。

## 2026-11-10（UI 改用 Slint，分支 `slint-rego`，基於 `1c6f0cf`）

- 移除 eframe／egui，畫面改以 Slint 1.18.1（GPLv3 授權選項、fluent 樣式、winit＋femtovg）宣告於 `app/ui/*.slint`；原 6300 行的 `ui.rs` 拆成 `app/src/ui/` 下的狀態模組與各頁模組，最大檔約 900 行。設定改為獨立系統視窗。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、20、57、24 項）。原 egui 畫面測試改寫為狀態邏輯測試，另以 `i-slint-backend-testing` 點擊實際元件：進階選單開啟與選項、批次表格雙擊切到主機頁、設定樹切換與收合。新增主機庫密碼檢查測試時發現並修正：上一次的錯誤訊息會殘留，讓修正後的密碼仍被擋下。
- 以 `--features capture` 加 `MISF_FIXTURE=1` 假資料截圖目視確認：主機頁、批次頁、設定視窗（深色中文、淺色英文）、主機編輯與 SSH 金鑰對話框、建立主機庫對話框。批次說明原本提到已移除的「表格左上角 ⇅」，改為指向設定頁。
- 尚未完成或未驗證：未在執行中的 app 實際操作拖曳排序、懸停提示時機、右鍵複製命令、閒置自動鎖定與視窗大小記憶；批次明細視窗不支援 Esc 關閉；打包腳本與第三方授權（移除 egui 字型、加入 Slint）尚未更新，此分支在功能對齊前不打包。

## 2026-11-09（設定視窗分類樹，基於 `69cd0ec`）

- 設定左側改為分類樹：「偏好設定」可收合，底下分為外觀與主機清單、安全性、批次作業；右側標題顯示「偏好設定 › 頁名」，各頁以分隔線區段分組。`MISF_OPEN_SETTINGS` 新增 `security`、`batch`。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、18、57、24 項）。新增 UI 測試：子頁縮排、點子頁切換與標題路徑、分支收合與展開、從其他頁點分支開啟第一頁。
- 以 `--features capture` 截圖目視確認安全性與批次作業頁的樹狀導覽與區段分隔線（測試資料夾無主機庫，畫面上同時有建立主機庫對話框）。

## 2026-11-09（閒置自動鎖定，基於 `ee69d1b`）

- 設定 > 偏好設定新增「自動鎖定」：永不（預設）或閒置 1～1440 分鐘後鎖定。只計使用者輸入；作業執行中不鎖定。時間到先清除主機快取與升級／重開機確認、dist-upgrade 報告等視窗，再送出 `vault_lock`，稽核為 `vault_auto_lock`。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、17、57、24 項）。新增：自動鎖定計時、輸入與忙碌重新計時、到時清除並送出鎖定、未加密不動作；偏好設定往返與超出範圍值視為永不。
- 視窗最小化後還原時，先判斷是否超時再處理輸入（有單元測試）；未在執行中的 app 實際等待閒置鎖定，也未確認最小化期間 eframe 是否仍會依排程重繪。

## 2026-11-09（右鍵複製命令、進階操作選單、批次雙擊跳主機頁，基於 `10ff78c`）

- 主機頁的升級、dist-upgrade、重新開機收進「進階操作」選單；選單開關由 App 自行保存，項目的滑鼠提示與右鍵複製命令仍可使用。批次表格雙擊主機名稱切到主機頁，右鍵選單提供「前往主機頁面」與（視窗模式）「顯示套件明細」。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、16、57、24 項）。新增 UI 測試：選單內右鍵複製到剪貼簿且選單不關閉、左鍵選項目開啟確認並關閉選單、點外面關閉；批次右鍵開啟明細視窗、雙擊切到主機頁。
- 未在執行中的 app 目視確認選單外觀。後續：⏷（U+23F7）在載入的字型中沒有字形而顯示為方框，已改為以 `Atom::paint` 繪製三角形（開啟時朝上）。

## 2026-11-09（kept back 報告、dist-upgrade、sudo 重啟狀態、輸出標題框，基於 `6519027`）

- APT upgrade 的 kept back 套件存入 `kept_back`，並以唯讀 `apt-get -s dist-upgrade` 模擬變動；操作結束時 UI 彈出報告。新增 `dist_upgrade`（單台與批次）。重啟狀態一般帳號改為整段 `sh -c` 放進 sudo。執行輸出與 LOG 改用依顯示寬度對齊的星號框標題。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、14、57、24 項）。新增或更新：kept back 與模擬計畫解析、sudo upgrade 觸發模擬且命令順序與確認清單一致、dist-upgrade 只執行自己的命令且非 APT 不碰套件、一般帳號經 sudo 讀重啟狀態、UI 批次結束產生報告、LOG 框線格式。
- 未在執行中的 app 目視確認報告視窗與按鈕；未在真實主機執行 dist-upgrade 或驗證 sudo-rs 下的狀態命令。

## 2026-11-09（完整版 LOG 與分段格式，基於 `c5bfe1e`）

- LOG 改為完整內容：請求與回應全文，只遮蔽密碼、私鑰、密語、PIN 欄位；每次遠端操作有 `********` 標題，每台主機一個區塊到 `END OF <名稱>`，含 ID、帳號@位址、指紋與遠端原始輸出。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、14、56、23 項）。測試涵蓋：巢狀機密欄位遮蔽但 ID／位址／帳號保留、主機區塊格式、UI 中解鎖與批次流程的標題與區塊、匯出與 16 MiB 上限。
- 未在執行中的 app 目視確認；未用真實主機的批次升級輸出檢查匯出檔。

## 2026-11-09（記憶體 LOG 與匯出，基於 `c68612e`）

- 新增 `app/src/session_log.rs`，在「設定 > LOG」匯出；稽核新增 `log_export`。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、14、56、23 項）。新增：請求只保留非機密欄位、結果附輸出、超過上限捨棄最舊、匯出檔內容，以及 UI 層以解鎖密碼／主機密碼送入後 LOG 不含任何機密。
- 未在執行中的 app 目視確認 LOG 設定頁與存檔對話框。

## 2026-11-09（APT 重啟提示與重新開機，基於 `aeb97d1`）

- APT root 在檢查／升級後執行唯讀 `apt::STATUS`（reboot-required＋`needrestart -r l -b`）；一般帳號不執行。新增 `reboot` 請求（`shutdown -r +1`，只限 APT），UI 兩次確認。SQLite v6：`restart_status`、`restart_items`。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、10、56、23 項）。新增：`parse_restart` 解析、reboot 命令形式、root 升級後讀取重啟狀態且命令順序與確認清單一致、一般帳號不執行 needrestart、reboot 只跑一條固定命令且拒絕非 APT 主機、SQLite 往返含重啟狀態、舊設定檔預設顯示新欄位。
- 未在執行中的 app 目視確認「重啟提示」分頁、警示列與兩次確認視窗；未在真實 Ubuntu 主機驗證 `needrestart -b` 輸出格式與 `shutdown -r +1`。

## 2026-11-08（批次逐台儲存與失敗詢問，基於 `8d40cd3`）

- 新增 `core/src/batch.rs`：每台完成即儲存並回報；失敗時暫停開始新主機，等待繼續／中斷；中斷後未開始的主機列在 `skipped`，稽核記為 cancelled。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、10、54、21 項），新增 `batch_reports_each_host_and_an_abort_skips_hosts_not_started`（12 台、第一台指紋不符→中斷），連跑 10 次皆通過。
- 未在執行中的 app 目視確認失敗詢問視窗與表格逐列更新；skipped→cancelled 的稽核紀錄沒有專屬測試；未對真實主機實測。

## 2026-11-08（拆分 package_manager 模組，基於 `630cda6`）

- 純重構：`ssh.rs` 只留 SSH 傳輸；套件管理移到 `core/src/package_manager/`（apt／dnf／yum／pkg／rpm／detect／operation）。命令字串內容未改，只換位置。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、10、54、20 項），新增 root／sudo 命令形式、ID 往返、套件管理器選擇與各管理器失敗判斷的單元測試。
- 未對真實主機重新實測。

## 2026-11-08（測試連線資訊，基於 `2669c10`）

- fmt、Clippy（`-D warnings`）通過；`cargo test --workspace` 全部通過（22、10、46、20 項）。
- 整合測試在 loopback 伺服器驗證：成功時記錄伺服器版本、kex、cipher、MAC、主機金鑰 `ssh-ed25519` 與指紋、密碼／公鑰登入；指紋不符與密碼錯誤時狀態為 failed、仍保留已協商資訊、未記錄登入方式，錯誤不含密碼。
- SQLite 往返測試涵蓋 `connection_tests`（v5）。
- 未在執行中的 app 目視確認「連線資訊」分頁排版；未對真實伺服器實測。

## 2026-11-08（移除 RSA，基於 `b63645b`）

- fmt、Clippy（`-D warnings`）通過；`cargo test --workspace` 全部通過（22、9、46、20 項），新增 `rsa_private_keys_are_rejected_in_both_formats`。
- `cargo audit`：退出碼 0，RUSTSEC-2023-0071 不再命中；`cargo tree -i rsa` 找不到套件。
- 未對只提供 RSA 主機金鑰的真實伺服器實測錯誤訊息。

## 2026-11-07（commit `af2c75e`，強制加密）

- `cargo fmt --all -- --check`：通過。
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`：通過。
- `cargo test --locked --workspace`：全部通過（各測試目標 22、9、45、20 項）。
- `.\package.ps1`：成功；`dist\MISF\misf.exe` 與 `target\release\misf.exe` 時間 2026-11-07 00:09:32、雜湊相同。
- 未在執行中的 app 目視確認新的建立加密主機庫畫面。

## 較早紀錄

- 2026-10-29 LUKSbox 整合時：完整工作區 19 項測試與 Clippy 通過（見 `luksbox-integration.md`）。
- 2026-10-30 依賴更新後：Windows 工作區 62 項測試通過（見 `dependency-update-2026-10-30.md`）。
