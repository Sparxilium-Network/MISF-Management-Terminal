# 全工作區檢查結果

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
