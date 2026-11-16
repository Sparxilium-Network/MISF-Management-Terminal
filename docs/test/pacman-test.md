# pacman 測試清單

> 最後修改：2026-11-17 ｜ 基於 commit：`546e058`

與 pacman（Arch Linux）有關的所有測試。選取參數的說明見 [總覽](README.md)；映像、測試套件、特殊行為與缺口的細節見 [`sshtest/pacman/README.md`](../../sshtest/pacman/README.md)。

## 怎麼跑

```powershell
# 單元測試（8 項：pacman 專屬 3 項，加上涵蓋所有套件管理器的 5 項）
cargo test -p misf-core --lib -- package_manager::pacman package_manager::tests package_manager::operation package_manager::detect
```

```powershell
# core 整合測試（2 項，約 1 秒）；兩項名稱都含 pacman，直接過濾即可
cargo test -p misf-core --test workflow -- pacman
```

```powershell
# 真實主機（23 項，要 Docker；不含慢速測試約 40 秒）
cargo test -p misf-sshtest --test pacman -- --ignored
```

基底映像預設 `archlinux:latest`。Arch 是滾動發行，隔一段時間重建映像，pacman 的版本與輸出可能改變。

## 單元測試

| 檔案 | 測試 | 驗證什麼 |
| --- | --- | --- |
| `core/src/package_manager/pacman.rs` | `listing_rows_split_into_versions_and_unknown_text_is_refused` | `checkupdates` 的 `名稱 舊版 -> 新版` 拆成三欄，含 epoch 與 `[ignored]` 標記；空輸出是沒有待更新；`==> ERROR:` 之類的文字判為無法解析 |
| | `failures_the_exit_code_hides_are_seen` | `==> ERROR: Cannot fetch updates` 判為索引更新失敗；`checkupdates` 的退出碼 0 與 2（沒有待更新）都算列出成功；升級輸出的 `error:` 行判為升級失敗 |
| | `checks_never_sync_the_system_databases` | 檢查用的兩個命令都不含 `pacman -S` 與 sudo |

另有涵蓋所有套件管理器的 5 項，見 [總覽的共用測試](README.md#涵蓋所有套件管理器的單元測試)。

## core 整合測試（`core/tests/workflow.rs`）

用 loopback 模擬 SSH 伺服器與 Arch Linux 主機（`PACMAN_HOST`）。

| 測試 | 驗證什麼 |
| --- | --- |
| `pacman_checks_without_sudo_or_a_system_sync_and_upgrades_through_sudo` | 檢查不經 sudo、不同步系統資料庫（索引更新依主機設定使用 PTY，清單不用）；升級經 sudo 輸入密碼；回報已升級與留下的套件 |
| `a_failed_pacman_sync_says_what_the_host_needs_and_lists_nothing` | `checkupdates` 同步失敗時存成失敗、不列出套件，錯誤訊息提到需要 pacman-contrib |

## 真實主機（`sshtest/pacman/`）

真的 sshd、sudo、pacman、`checkupdates` 與 fakeroot。帳號欄：`Admin` 經要密碼的 sudo，`Root` 直接執行，`Deploy` 經免密碼 sudo。GAP 欄是測試裡斷言的缺口，編號只在 pacman 內有效，內容見 [`sshtest/pacman/README.md`](../../sshtest/pacman/README.md#測試發現的缺口)。

### `upgrade.rs`：正常升級

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_check_leaves_the_system_databases_alone_and_the_upgrade_goes_through_sudo` | 基準：檢查不用 sudo、不動系統資料庫；升級經 sudo、連帶安裝相依；`backup` 設定檔保留並產生 `.pacnew` | Admin | |
| `root_upgrades_without_sudo_or_a_terminal_and_has_no_dist_upgrade` | root 不經 sudo、不用 PTY；沒有 dist-upgrade | Root | |
| `passwordless_sudo_upgrades_without_a_terminal` | 免密碼 sudo、不用 PTY 也能升級 | Deploy | |

### `failures.rs`：半途失敗或沒開始

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_failing_script_fails_the_run_though_every_package_went_in` | `post_upgrade`／`pre_upgrade` 失敗：整次存成失敗，但每個套件都已裝上 | Admin、Root | |
| `one_package_that_cannot_be_installed_blocks_every_upgrade` | 一個套件相依不滿足，整次升級都不做 | Admin | |
| `a_broken_repository_stops_before_anything_is_listed_or_installed` | 套件庫資料庫抓不到，不列出也不安裝；錯誤看不出原因 | Admin | 1 |
| `a_host_without_checkupdates_or_fakeroot_cannot_be_checked_and_is_told_why` | 缺 fakeroot、缺 `checkupdates` 時不能檢查，錯誤說明要裝什麼 | Admin | |
| `a_package_file_that_cannot_be_read_fails_the_upgrade_and_changes_nothing` | 套件檔不見、套件檔被截斷，升級失敗且不變更任何套件 | Admin、Root | |
| `pacman_killed_half_way_leaves_its_lock_and_later_upgrades_cannot_start` | 升級途中 pacman 被殺掉，留下 `db.lck`，之後的升級都無法開始 | Admin | 2、3 |
| `an_upgrade_while_another_pacman_holds_the_lock_fails_and_works_afterwards` | 另一個 pacman 佔著鎖時失敗，鎖釋放後可以升級 | Admin | |

### `output.rs`：不能直接相信的輸出

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `garbage_from_a_package_script_is_cleaned_and_does_not_fail_the_upgrade` | 控制字元、非 UTF-8、沒有結尾的跳脫序列等不正常的輸出 | Admin、Root | 4 |
| `a_script_line_that_looks_like_a_pacman_error_fails_an_upgrade_that_worked` | 套件腳本印出 `error:` 開頭的一行，成功的升級被存成失敗 | Root | 5 |
| `root_ignores_a_package_script_that_prints_the_sudo_prompt_marker` | root：輸出出現 sudo 提示標記，不送密碼 | Root | |
| `the_marker_after_sudo_took_the_password_reads_as_a_wrong_password` | 要密碼的 sudo：升級途中出現標記，被當成密碼錯誤並斷線 | Admin | 6 |
| `with_passwordless_sudo_the_marker_is_answered_but_no_script_can_read_it` | 免密碼 sudo：MISF 回應了標記，但密碼到不了套件腳本 | Deploy | |

### `prompts.rs`：套件腳本要人回答

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_dialog_cannot_open_on_the_dumb_terminal_and_the_upgrade_goes_on` | `whiptail` 在 `TERM=dumb` 下開不起來，升級繼續 | Admin、Root | |
| `a_question_on_the_scripts_input_gets_end_of_input_at_once` | 腳本從標準輸入 `read`，立刻讀到結尾，不會卡住 | Admin、Root | |
| `a_dialog_drawn_on_the_terminal_keeps_the_upgrade_waiting` | 腳本在 `/dev/tty` 畫出對話框，升級一直等待 | Admin、Root | 7 |
| `a_question_asked_on_the_terminal_keeps_the_upgrade_waiting` | 腳本在 `/dev/tty` 等回答，升級一直等待 | Admin、Root | 7 |

### `connection.rs`：升級途中斷線

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_cut_connection_with_a_pty_fails_the_run_while_root_s_pacman_finishes` | root 有 PTY 時斷線：MISF 存成失敗，遠端的 pacman 做完 | Root | 8 |
| `a_cut_connection_without_a_pty_leaves_pacman_s_lock_behind` | 沒有 PTY 時斷線：pacman 來不及刪鎖檔，留下 `db.lck` | Admin、Root | 3 |
| `a_cut_connection_under_sudo_with_a_pty_cuts_the_running_script_short` | 經 sudo 且有 PTY 時斷線：執行中的腳本被結束，套件仍算已安裝 | Admin | 9 |
| `a_silently_dead_link_is_given_up_on_after_minutes_of_keepalives`（慢，約 4 分鐘） | 網路無聲中斷，約 210 秒後才放棄；要設 `MISF_SSHTEST_SLOW=1` | Root | 8 |
