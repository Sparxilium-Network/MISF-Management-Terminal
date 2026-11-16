# APT 測試清單

> 最後修改：2026-11-17 ｜ 基於 commit：`546e058`

與 APT（Debian／Ubuntu）有關的所有測試。選取參數的說明見 [總覽](README.md)；映像、測試套件與缺口的細節見 [`sshtest/apt/README.md`](../../sshtest/apt/README.md)。

## 怎麼跑

```powershell
# 單元測試（12 項：APT 專屬 7 項，加上涵蓋所有套件管理器的 5 項）
cargo test -p misf-core --lib -- package_manager::apt reboot:: phased package_manager::tests package_manager::operation package_manager::detect
```

```powershell
# core 整合測試（10 項，約 1 秒）
$apt = @(
  'password_pty_sudo_and_packages_work_end_to_end'
  'sudo_upgrade_reports_installed_and_held_back_packages'
  'root_upgrade_runs_without_sudo'
  'root_check_executes_apt_without_sudo'
  'regular_accounts_read_restart_status_through_sudo'
  'dist_upgrade_runs_only_its_own_command_and_clears_kept_back_packages'
  'reboot_runs_one_fixed_command_and_only_on_apt_hosts'
  'failed_index_update_never_starts_an_upgrade'
  'apt_update_failure_never_lists_stale_packages'
  'malformed_store_is_preserved_and_unparseable_apt_output_is_not_success'
)
cargo test -p misf-core --test workflow -- --exact @apt
```

```powershell
# 真實主機（18 項，要 Docker；不含慢速測試約 45 秒）
cargo test -p misf-sshtest --test apt -- --ignored
```

基底映像預設 `debian:13`，改用 Ubuntu：`$env:MISF_SSHTEST_APT_BASE = 'ubuntu:26.04'`。

## 單元測試

| 檔案 | 測試 | 驗證什麼 |
| --- | --- | --- |
| `core/src/package_manager/apt.rs` | `listing_rows_split_into_versions_and_unknown_text_is_refused` | `apt list --upgradable` 的每一行拆成名稱、目前版本、候選版本；看不懂的文字不當成套件，整份清單判為無法解析 |
| | `failed_repositories_are_seen_even_with_zero_exit` | `apt-get update` 退出碼 0 但有套件庫抓不到時，仍判為索引更新失敗 |
| | `kept_back_packages_and_the_dist_upgrade_plan` | 讀出升級後被 kept back 的套件，以及 dist-upgrade 模擬的升級、安裝、移除計畫 |
| | `phased_upgrades_are_read_under_old_and_new_wording` | 新舊兩種 APT 措辭的 phased updates 都讀得出來 |
| | `restart_status_reads_reboot_required_and_needrestart_batch_output` | 讀出 `/var/run/reboot-required` 與 `needrestart -b` 的核心、服務重啟狀態 |
| `core/src/reboot.rs` | `root_reboots_directly_and_others_answer_the_sudo_marker` | 重新開機命令：root 直接執行，一般帳號經 sudo 提示標記 |
| `core/src/sqlite.rs` | `a_check_reads_back_phased_packages_but_no_dist_upgrade_plan` | 檢查結果的 phased 套件存進 SQLite 後讀得回來，檢查不帶 dist-upgrade 計畫 |

另有涵蓋所有套件管理器的 5 項，見 [總覽的共用測試](README.md#涵蓋所有套件管理器的單元測試)。

## core 整合測試（`core/tests/workflow.rs`）

用 loopback 模擬 SSH 伺服器與 Debian 13 主機（`APT_HOST`），回應是寫好的 APT 輸出。

| 測試 | 驗證什麼 |
| --- | --- |
| `password_pty_sudo_and_packages_work_end_to_end` | 測試連線回報的系統、演算法與指紋；檢查列出待更新套件與 phased 套件；索引更新與重啟狀態經 sudo 並使用 PTY；輸出沒有密碼、提示標記與控制字元；執行的命令與確認畫面列出的相同 |
| `sudo_upgrade_reports_installed_and_held_back_packages` | 經 sudo 升級：回報已升級與 kept back 的套件，並模擬 dist-upgrade 計畫（不實際變更） |
| `root_upgrade_runs_without_sudo` | root 升級不經 sudo |
| `root_check_executes_apt_without_sudo` | root 檢查不經 sudo |
| `regular_accounts_read_restart_status_through_sudo` | 一般帳號經 sudo 讀取重啟狀態 |
| `dist_upgrade_runs_only_its_own_command_and_clears_kept_back_packages` | dist-upgrade 只執行自己的命令，完成後沒有 kept back；沒有 kept back 時不跑模擬；DNF 主機不支援、在動到套件前停止 |
| `reboot_runs_one_fixed_command_and_only_on_apt_hosts` | 重新開機只執行一個固定命令；root 不經 sudo；DNF 主機拒絕 |
| `failed_index_update_never_starts_an_upgrade` | 索引更新失敗時不執行升級 |
| `apt_update_failure_never_lists_stale_packages` | 索引更新失敗時不列出舊的待更新清單 |
| `malformed_store_is_preserved_and_unparseable_apt_output_is_not_success` | 損壞的主機庫保留原檔；無法解析的 APT 輸出不算成功 |

`batches_reject_mixed_or_undetected_package_managers` 也用到 APT 主機，列在 [DNF 清單](dnf-test.md)。

## 真實主機（`sshtest/apt/`）

真的 sshd、sudo、apt、dpkg、debconf 與 needrestart。帳號欄：`Admin` 經要密碼的 sudo，`Root` 直接執行，`Deploy` 經免密碼 sudo。GAP 欄是測試裡斷言的缺口，編號只在 APT 內有效，內容見 [`sshtest/apt/README.md`](../../sshtest/apt/README.md#測試發現的缺口)。

### `upgrade.rs`：正常升級

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_clean_upgrade_through_sudo_reports_packages_and_the_reboot_request` | 基準：檢查、升級、真實的 needrestart 輸出、`reboot-required` | Admin | |
| `questions_apt_would_ask_are_answered_without_anyone_there` | debconf 的 critical 問題用預設值，管理者改過的 conffile 保留，都不會跳出詢問 | Admin | |
| `kept_back_packages_get_a_simulated_plan_and_a_dist_upgrade_installs_them` | `upgrade` 把套件 kept back，模擬 dist-upgrade 計畫，再以 dist-upgrade 裝上 | Admin | |

### `failures.rs`：半途失敗或沒開始

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_failing_package_script_fails_the_upgrade_and_leaves_it_half_configured` | postinst 失敗：一個套件完成、一個停在 half-configured；之後再檢查看起來是最新的 | Admin、Root | 1 |
| `a_broken_repository_stops_before_anything_is_listed_or_installed` | 套件庫索引抓不到，不列出也不安裝 | Admin | |
| `a_package_file_that_cannot_be_fetched_fails_the_upgrade_and_changes_nothing` | 套件檔不見、套件檔被截斷，升級失敗且不變更任何套件 | Admin、Root | |
| `dpkg_killed_half_way_fails_the_upgrade_and_the_damage_is_not_seen_again` | 升級途中 dpkg 被殺掉；之後再檢查看不到損壞 | Admin | 1 |
| `an_upgrade_while_another_apt_holds_the_lock_fails_and_works_afterwards` | 另一個 apt 佔著鎖時失敗，鎖釋放後可以升級 | Admin | |

### `output.rs`：不能直接相信的輸出

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `garbage_from_a_package_script_is_cleaned_and_does_not_fail_the_upgrade` | ANSI 顏色、視窗標題、`\r` 進度、非 UTF-8、NUL、退格、十萬字元的一行、沒有結尾的跳脫序列 | Admin、Root | 2 |
| `a_script_line_that_looks_like_an_apt_error_fails_an_upgrade_that_worked` | 套件腳本印出 `E:` 開頭的一行，成功的升級被存成失敗 | Root | 3 |
| `root_ignores_a_package_script_that_prints_the_sudo_prompt_marker` | root：輸出出現 sudo 提示標記，不送密碼 | Root | |
| `the_marker_after_sudo_took_the_password_reads_as_a_wrong_password` | 要密碼的 sudo：升級途中出現標記，被當成密碼錯誤並斷線 | Admin | 4 |
| `with_passwordless_sudo_the_marker_gets_the_password_typed_into_the_script` | 免密碼 sudo：標記被當成 sudo 提示，密碼送進套件腳本（有無 PTY） | Deploy | 5 |

### `prompts.rs`：套件腳本要人回答

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_dialog_cannot_open_on_the_dumb_terminal_and_fails_at_once` | `whiptail` 在 `TERM=dumb` 下開不起來，立刻失敗 | Admin、Root | |
| `a_drawn_dialog_keeps_the_upgrade_waiting_and_outlives_the_stop` | 強制 `TERM=xterm` 畫出對話框，升級一直等待；中斷後遠端仍卡住並持有鎖 | Admin、Root | 1、6、7 |
| `a_script_waiting_for_an_answer_blocks_until_the_connection_goes` | 腳本 `read` 等回答，直到連線結束 | Admin、Root | 6、7 |

### `connection.rs`：升級途中斷線

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_cut_connection_fails_the_run_while_the_host_finishes_the_upgrade` | 升級途中連線被切斷：MISF 存成失敗，遠端照常升級完成 | Admin、Root | 8 |
| `a_silently_dead_link_is_given_up_on_after_minutes_of_keepalives`（慢，約 4 分鐘） | 網路無聲中斷，約 210 秒後才放棄；要設 `MISF_SSHTEST_SLOW=1` | Admin | 8 |
