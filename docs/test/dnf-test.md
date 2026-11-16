# DNF 測試清單

> 最後修改：2026-11-17 ｜ 基於 commit：`546e058`

與 DNF（Rocky Linux、AlmaLinux 等 Enterprise Linux 9）有關的所有測試。選取參數的說明見 [總覽](README.md)；映像、測試套件與缺口的細節見 [`sshtest/dnf/README.md`](../../sshtest/dnf/README.md)。

## 怎麼跑

```powershell
# 單元測試（8 項：DNF 專屬 3 項，加上涵蓋所有套件管理器的 5 項）
cargo test -p misf-core --lib -- package_manager::dnf package_manager::rpm package_manager::tests package_manager::operation package_manager::detect
```

```powershell
# core 整合測試（6 項，約 1 秒）
$dnf = @(
  'sudo_dnf_upgrade_parses_wrapped_rows_and_reports_held_back_packages'
  'recorded_manager_mismatch_and_unsupported_systems_never_touch_packages'
  'batches_reject_mixed_or_undetected_package_managers'
  'chosen_package_manager_overrides_detection_only_when_installed'
  'dist_upgrade_runs_only_its_own_command_and_clears_kept_back_packages'
  'reboot_runs_one_fixed_command_and_only_on_apt_hosts'
)
cargo test -p misf-core --test workflow -- --exact @dnf
```

```powershell
# 真實主機（22 項，要 Docker；不含慢速測試約 45 秒）
cargo test -p misf-sshtest --test dnf -- --ignored
```

基底映像預設 `rockylinux/rockylinux:9`，改用 AlmaLinux：`$env:MISF_SSHTEST_DNF_BASE = 'almalinux:9'`。Fedora（dnf5）目前不能用，原因見 sshtest 說明的 GAP 11。

## 單元測試

DNF 的清單解析放在 `rpm.rs`，`dnf.rs` 只有命令。

| 檔案 | 測試 | 驗證什麼 |
| --- | --- | --- |
| `core/src/package_manager/dnf.rs` | `listings_print_the_marker_the_parser_splits_on` | 兩個清單命令（root 與 sudo）都印出解析器用來分段的已安裝版本標記 |
| `core/src/package_manager/rpm.rs` | `rows_survive_wrapping_and_obsoletes_and_pick_installed_versions` | `check-update` 被折行的長名稱、`Obsoleting Packages` 區段都能正確解析，目前版本取自已安裝清單；缺少標記或有 `Error:` 行時判為無法解析 |
| | `skipped_repositories_and_exit_codes` | `Ignoring repositories`、`Failed to download metadata` 判為索引更新失敗；`check-update` 的退出碼 0 與 100 都算列出成功 |

另有涵蓋所有套件管理器的 5 項，見 [總覽的共用測試](README.md#涵蓋所有套件管理器的單元測試)。

## core 整合測試（`core/tests/workflow.rs`）

用 loopback 模擬 SSH 伺服器與 Rocky Linux 9.5 主機（`DNF_HOST`）。後三項同時測 APT 或 apk，也列在對應的清單。

| 測試 | 驗證什麼 |
| --- | --- |
| `sudo_dnf_upgrade_parses_wrapped_rows_and_reports_held_back_packages` | 經 sudo 升級：解析折行的套件列，回報已升級與留下的套件；索引更新、兩次清單與升級都經 sudo 與 PTY；完全不執行 apt 命令 |
| `recorded_manager_mismatch_and_unsupported_systems_never_touch_packages` | 紀錄是 apt 但偵測到 dnf 時拒絕並要求重新測試連線；只有 yum 的 CentOS 7 回報不支援；兩者都只執行偵測 |
| `batches_reject_mixed_or_undetected_package_managers` | 批次裡 APT 與 DNF 主機混在一起、或有尚未偵測的主機時拒絕；只有 DNF 主機的批次可以檢查並保存結果 |
| `chosen_package_manager_overrides_detection_only_when_installed` | DNF 主機上選了沒安裝的 apt 時拒絕，只執行偵測（前半段是 apk，見 [apk 清單](apk-test.md)） |
| `dist_upgrade_runs_only_its_own_command_and_clears_kept_back_packages` | 後半段：DNF 主機沒有 dist-upgrade，在動到套件前停止（前半段是 APT） |
| `reboot_runs_one_fixed_command_and_only_on_apt_hosts` | 後半段：DNF 主機不能從 MISF 重新開機（前半段是 APT） |

## 真實主機（`sshtest/dnf/`）

真的 sshd、sudo、dnf 與 rpm。帳號欄：`Admin` 經要密碼的 sudo，`Root` 直接執行，`Deploy` 經免密碼 sudo。GAP 欄是測試裡斷言的缺口，編號只在 DNF 內有效，內容見 [`sshtest/dnf/README.md`](../../sshtest/dnf/README.md#測試發現的缺口)。

### `upgrade.rs`：正常升級

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_clean_upgrade_through_sudo_reports_packages_and_keeps_changed_config` | 基準：檢查、升級、連帶安裝相依；管理者改過的設定檔保留，新版存成 `.rpmnew` | Admin | |
| `root_upgrades_without_sudo_or_a_terminal_and_has_no_dist_upgrade` | root 不經 sudo、不用 PTY；DNF 沒有 dist-upgrade | Root | |

### `failures.rs`：半途失敗或沒開始

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_failing_post_script_is_only_a_line_in_the_output` | `%post` 失敗仍存成成功，只在輸出留一行 | Admin | 1 |
| `a_failing_pre_script_fails_the_upgrade_and_the_package_is_offered_again` | `%pre` 失敗：交易做到一半，失敗的套件再檢查時仍待更新 | Admin、Root | |
| `one_package_that_cannot_be_installed_blocks_every_upgrade` | 一個套件相依不滿足，整次升級都不做 | Admin | |
| `a_broken_repository_stops_before_anything_is_listed_or_installed` | 套件庫中繼資料抓不到，不列出也不安裝 | Admin | |
| `a_repository_dnf_may_skip_is_missed_by_the_check_and_fails_a_good_upgrade` | 設了 `skip_if_unavailable` 的套件庫抓不到：檢查沒發現，升級成功卻存成失敗 | Admin | 2、3 |
| `a_package_file_that_cannot_be_read_fails_the_upgrade_and_changes_nothing` | 套件檔不見、套件檔被截斷，升級失敗且不變更任何套件 | Admin、Root | |
| `dnf_killed_half_way_leaves_a_duplicate_that_is_not_seen_again` | 升級途中 dnf 被殺掉，留下新舊兩版並存，之後看不到 | Admin | 4、5 |
| `an_upgrade_waits_for_another_dnf_to_release_the_lock` | 另一個 dnf 佔著鎖時排隊等待，之後完成 | Admin | |

### `output.rs`：不能直接相信的輸出

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `garbage_from_a_package_script_is_cleaned_and_does_not_fail_the_upgrade` | 控制字元、非 UTF-8、沒有結尾的跳脫序列等不正常的輸出 | Admin、Root | 6 |
| `a_script_line_that_looks_like_a_dnf_error_fails_an_upgrade_that_worked` | 套件腳本印出 `Error:` 開頭的一行，成功的升級被存成失敗 | Root | 3 |
| `root_ignores_a_package_script_that_prints_the_sudo_prompt_marker` | root：輸出出現 sudo 提示標記，不送密碼 | Root | |
| `the_marker_after_sudo_took_the_password_reads_as_a_wrong_password` | 要密碼的 sudo：升級途中出現標記，被當成密碼錯誤並斷線 | Admin | 7 |
| `with_passwordless_sudo_the_marker_is_answered_but_no_script_can_read_it` | 免密碼 sudo：MISF 回應了標記，但 rpm 不給腳本標準輸入，密碼到不了腳本 | Deploy | |

### `prompts.rs`：套件腳本要人回答

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_dialog_cannot_open_on_the_dumb_terminal_and_the_upgrade_goes_on` | `whiptail` 在 `TERM=dumb` 下開不起來，升級繼續（`%post` 失敗只留一行） | Admin、Root | 1 |
| `a_question_on_the_scripts_input_gets_end_of_input_at_once` | 腳本從標準輸入 `read`，立刻讀到結尾，不會卡住 | Admin、Root | |
| `a_dialog_drawn_on_the_terminal_keeps_the_upgrade_waiting` | 腳本在 `/dev/tty` 畫出對話框，升級一直等待 | Admin、Root | 8 |
| `a_question_asked_on_the_terminal_keeps_the_upgrade_waiting` | 腳本在 `/dev/tty` 等回答，升級一直等待 | Admin、Root | 8 |

### `connection.rs`：升級途中斷線

| 測試 | 情境與驗證 | 帳號 | GAP |
| --- | --- | --- | --- |
| `a_cut_connection_fails_the_run_while_root_s_dnf_finishes_the_upgrade` | root 升級途中斷線：MISF 存成失敗，遠端的 dnf 做完 | Root | 9 |
| `a_cut_connection_under_sudo_with_a_pty_cuts_the_running_script_short` | 經 sudo 且有 PTY 時斷線：執行中的腳本被結束，套件仍算已安裝 | Admin | 10 |
| `a_silently_dead_link_is_given_up_on_after_minutes_of_keepalives`（慢，約 4 分鐘） | 網路無聲中斷，約 210 秒後才放棄；要設 `MISF_SSHTEST_SLOW=1` | Root | 9 |
