# apk 測試清單

> 最後修改：2026-11-17 ｜ 基於 commit：`546e058`

與 apk（Alpine Linux）有關的所有測試。選取參數的說明見 [總覽](README.md)；映像、測試套件、特殊行為與缺口的細節見 [`sshtest/apk/README.md`](../../sshtest/apk/README.md)。

## 怎麼跑

```powershell
# 單元測試（7 項：apk 專屬 2 項，加上涵蓋所有套件管理器的 5 項）
cargo test -p misf-core --lib -- package_manager::apk package_manager::tests package_manager::operation package_manager::detect
```

```powershell
# core 整合測試（2 項，約 1 秒）
$apk = @(
  'apk_upgrades_as_root_and_an_unfetched_index_fails_despite_exit_zero'
  'chosen_package_manager_overrides_detection_only_when_installed'
)
cargo test -p misf-core --test workflow -- --exact @apk
```

```powershell
# 真實主機（22 項，要 Docker；不含慢速測試約 70 秒）
cargo test -p misf-sshtest --test apk -- --ignored
```

基底映像預設 `alpine:3.24`（apk-tools 3）。apk-tools 2 沒有測過。

## 單元測試

| 檔案 | 測試 | 驗證什麼 |
| --- | --- | --- |
| `core/src/package_manager/apk.rs` | `listing_rows_split_into_versions_and_unknown_text_is_refused` | `apk list --upgradable` 的每一行拆成名稱、架構、目前版本、候選版本，跳過 `WARNING:` 行；空輸出是沒有待更新；`ERROR:` 或缺少 `[upgradable from: …]` 的行判為無法解析 |
| | `unreadable_indexes_are_failures_even_with_zero_exit` | 索引打不開、抓取失敗、`N errors;` 都判為索引更新失敗（即使 `apk update` 退出碼 0）；升級輸出的 `ERROR:` 行判為升級失敗 |

另有涵蓋所有套件管理器的 5 項，見 [總覽的共用測試](README.md#涵蓋所有套件管理器的單元測試)。

## core 整合測試（`core/tests/workflow.rs`）

用 loopback 模擬 SSH 伺服器與 Alpine 3.24 主機（`APK_HOST`）。這台模擬主機同時裝了 apt 與 apk。

| 測試 | 驗證什麼 |
| --- | --- |
| `apk_upgrades_as_root_and_an_unfetched_index_fails_despite_exit_zero` | root 升級不經 sudo，回報已升級與留下的套件；索引抓不到時即使退出碼 0 也存成失敗，不列出也不升級 |
| `chosen_package_manager_overrides_detection_only_when_installed` | 前半段：偵測優先選 apt，但使用者選了 apk，就執行 apk 的命令；偵測結果仍記為 apt（後半段是 DNF，見 [DNF 清單](dnf-test.md)） |

## 真實主機（`sshtest/apk/`）

真的 sshd、sudo 與 apk。帳號欄：`Admin` 經要密碼的 sudo，`Root` 直接執行，`Deploy` 經免密碼 sudo。GAP 欄是測試裡斷言的缺口，編號只在 apk 內有效，內容見 [`sshtest/apk/README.md`](../../sshtest/apk/README.md#測試發現的缺口)。

### `upgrade.rs`：正常升級

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_clean_upgrade_through_sudo_reports_packages_and_keeps_changed_config` | 基準：檢查、升級、連帶安裝相依；管理者改過的設定檔保留，新版存成 `.apk-new` | Admin | |
| `root_upgrades_without_sudo_or_a_terminal_and_has_no_dist_upgrade` | root 不經 sudo、不用 PTY；沒有 dist-upgrade | Root | |
| `passwordless_sudo_lists_without_sudo_and_upgrades_through_it` | 免密碼 sudo：索引更新與升級經 sudo，清單不經 sudo | Deploy | |

### `failures.rs`：半途失敗或沒開始

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_failing_script_fails_the_run_though_every_package_went_in` | `post-upgrade`／`pre-upgrade` 失敗：整次存成失敗，但每個套件都已裝上 | Admin、Root | |
| `one_package_that_cannot_be_installed_blocks_every_upgrade` | 一個套件相依不滿足，整次升級都不做 | Admin | |
| `a_missing_index_stops_before_anything_is_listed_or_installed` | 套件庫索引打不開（`apk update` 退出碼 0），檢查與升級都存成失敗 | Admin | |
| `an_index_without_a_trusted_signature_is_refused` | 索引沒有受信任的簽章，存成失敗且不安裝 | Root | |
| `apk_killed_half_way_fails_the_run_and_the_next_upgrade_finishes` | 升級途中 apk 被殺掉：這一輪什麼都沒記下，下次升級全部重做 | Admin | 6 |
| `while_another_apk_holds_the_lock_not_even_a_check_can_run` | 另一個 apk 佔著鎖時，連檢查都失敗；鎖隨程序釋放，之後可以升級 | Admin | |
| `a_package_file_that_cannot_be_read_fails_the_upgrade_after_the_others_went_in` | 套件檔不見：缺檔之前的套件已裝上，其餘再列為待更新；套件檔被截斷時不變更 | Admin、Root | |

### `output.rs`：不能直接相信的輸出

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `garbage_from_a_package_script_is_cleaned_and_does_not_fail_the_upgrade` | 控制字元、非 UTF-8、沒有結尾的跳脫序列等不正常的輸出 | Admin、Root | 2 |
| `a_script_line_that_looks_like_an_apk_error_is_only_text` | 套件腳本印出 `ERROR:` 行；apk 在腳本輸出前加 `* `，不會被誤判 | Root | |
| `root_ignores_a_package_script_that_prints_the_sudo_prompt_marker` | root：輸出出現 sudo 提示標記，不送密碼 | Root | |
| `the_marker_after_sudo_took_the_password_reads_as_a_wrong_password` | 要密碼的 sudo：升級途中出現標記，被當成密碼錯誤並斷線 | Admin | 1 |
| `with_passwordless_sudo_the_marker_is_answered_but_no_script_can_read_it` | 免密碼 sudo：MISF 回應了標記，但 apk 不給腳本輸入，密碼到不了腳本 | Deploy | |
| `a_question_on_the_scripts_input_gets_end_of_input_at_once` | 腳本從標準輸入 `read`，立刻讀到結尾，不會卡住 | Admin、Root | |

### `prompts.rs`：套件腳本要人回答

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_dialog_cannot_open_without_a_terminal_type_and_the_upgrade_goes_on` | 沒有 `TERM` 時 `whiptail` 開不起來，升級繼續 | Admin、Root | |
| `a_dialog_drawn_on_the_terminal_keeps_the_upgrade_waiting_and_is_asked_again` | 腳本在 `/dev/tty` 畫出對話框，升級一直等待；中斷後下次升級又卡在同一處 | Admin、Root | 4 |
| `a_question_asked_on_the_terminal_keeps_the_upgrade_waiting` | 腳本在 `/dev/tty` 等回答，升級一直等待 | Admin、Root | 4 |

### `connection.rs`：升級途中斷線

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_cut_connection_fails_the_run_while_root_s_apk_finishes_the_upgrade` | 沒有 PTY 時斷線：MISF 存成失敗，遠端的 apk 做完 | Root | 5 |
| `a_cut_connection_with_a_pty_ends_apk_and_the_whole_upgrade_is_offered_again` | 有 PTY 時斷線：apk 隨即結束、沒記下任何東西，整次升級再列為待更新 | Admin、Root | 3 |
| `a_silently_dead_link_is_given_up_on_after_minutes_of_keepalives`（慢，約 4 分鐘） | 網路無聲中斷，約 210 秒後才放棄；要設 `MISF_SSHTEST_SLOW=1` | Root | 5 |
