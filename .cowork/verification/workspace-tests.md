# 全工作區檢查結果

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
