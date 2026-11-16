# 測試清單總覽

> 最後修改：2026-11-17 ｜ 基於 commit：`546e058`

這個資料夾為每個套件管理器準備一份清單，列出與它有關的每一項測試：放在哪個檔案、測試名稱、驗證什麼，以及怎麼只跑這些測試。各層測試的分工見 [開發說明](../PROJECT-DEV.md#分工)；`sshtest/` 的映像、共用框架與 GAP 缺口的完整說明見 [`sshtest/README.md`](../../sshtest/README.md) 與各套件管理器資料夾的 `README.md`。

## 清單

| 套件管理器 | 清單 | 單元測試 | core 整合測試 | 真實主機（sshtest） |
| --- | --- | --- | --- | --- |
| APT | [apt-test.md](apt-test.md) | 7 項專屬 | 10 項 | 18 項 |
| DNF | [dnf-test.md](dnf-test.md) | 3 項專屬 | 6 項（3 項與 APT 共用） | 22 項 |
| pacman | [pacman-test.md](pacman-test.md) | 3 項專屬 | 2 項 | 23 項 |
| apk | [apk-test.md](apk-test.md) | 2 項專屬 | 2 項（1 項與 DNF 共用） | 22 項 |

每份清單的單元測試另外包含下面「共用的測試」裡涵蓋四個套件管理器的 5 項。不屬於任何一個套件管理器的測試（認證、儲存、批次、互動 shell 等）只列在本頁。

## 怎麼只跑一部分

`cargo test` 的選取方式：

```text
cargo test -p <crate> [--lib | --test <目標>] -- [過濾字串…] [--exact] [--ignored] [--list]
```

| 參數 | 作用 |
| --- | --- |
| `-p <crate>` | 只跑一個 crate：`misf-core`、`misf`、`misf-sshtest` |
| `--lib` | 只跑 crate 內各模組的單元測試 |
| `--test <目標>` | 只跑一個測試目標：`workflow`（core 整合測試），或 `apt`、`dnf`、`pacman`、`apk`（sshtest） |
| 過濾字串 | 寫在 `--` 之後，可以寫好幾個，名稱含有任一個就執行。比對的是完整名稱，包含模組路徑，例如 `package_manager::apt::tests::kept_back_packages_and_the_dist_upgrade_plan`、`connection::a_cut_connection_fails_the_run_while_the_host_finishes_the_upgrade` |
| `--exact` | 過濾字串必須與完整名稱完全相同。`workflow.rs` 的測試沒有模組路徑，完整名稱就是函式名稱 |
| `--ignored` | 只跑標為 `#[ignore]` 的測試，也就是 sshtest 的容器測試（要 Docker） |
| `--list` | 只列出會執行哪些測試，不執行 |

各層的名稱規則：

| 層級 | 選取方式 | 例子 |
| --- | --- | --- |
| 單元測試 | `-p misf-core --lib`，以模組路徑過濾 | `package_manager::apt`、`package_manager::rpm` |
| core 整合測試 | `-p misf-core --test workflow`，以測試名稱過濾 | `pacman`、或 `--exact` 加完整名稱 |
| UI 測試 | `-p misf`，以名稱過濾 | `batch` |
| 真實主機 | `-p misf-sshtest --test <管理器>`，加 `--ignored`；以檔案（模組）或名稱過濾 | `connection`、`a_cut_connection` |

`workflow.rs` 的測試名稱沒有統一的套件管理器前綴（大多數 APT 測試的名稱裡沒有 `apt`），所以各清單用 `--exact` 列出完整名稱。

## 例子：只跑 APT

三層各一個指令（PowerShell）。指令會用到的名稱與每一項的說明見 [apt-test.md](apt-test.md)。

```powershell
# 單元測試：APT 專屬的，加上涵蓋所有套件管理器的
cargo test -p misf-core --lib -- package_manager::apt reboot:: phased package_manager::tests package_manager::operation package_manager::detect
```

```powershell
# core 整合測試：以 APT 模擬主機執行的 10 項
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
# 真實主機：要先啟動 Docker
cargo test -p misf-sshtest --test apt -- --ignored
```

PowerShell 會把陣列的每個元素當成一個參數傳給 `cargo`。Linux／macOS 的 shell 直接把名稱接在 `--exact` 後面，以空白分隔。

其他套件管理器的做法相同，指令在各清單的「怎麼跑」一節。

## 更小的範圍

| 想跑什麼 | 指令 |
| --- | --- |
| 單元測試裡的一個模組 | `cargo test -p misf-core --lib -- package_manager::pacman` |
| sshtest 裡一個管理器的一類情境 | `cargo test -p misf-sshtest --test dnf -- --ignored connection` |
| sshtest 裡的一項 | `cargo test -p misf-sshtest --test apk -- --ignored --exact failures::an_index_without_a_trusted_signature_is_refused` |
| sshtest 四個管理器裡同名的測試 | `cargo test -p misf-sshtest -- --ignored garbage_from_a_package_script` |
| 包含慢速測試（網路無聲中斷，每個管理器約 4 分鐘） | 先設 `$env:MISF_SSHTEST_SLOW = '1'`，再執行上面的指令；用完 `Remove-Item Env:MISF_SSHTEST_SLOW` |
| 先看會跑哪些 | 任一指令最後加上 `--list` |

sshtest 的基底映像可用 `MISF_SSHTEST_<管理器>_BASE` 換掉，例如 `MISF_SSHTEST_APT_BASE=ubuntu:26.04`，見 [`sshtest/README.md`](../../sshtest/README.md#執行)。

## 共用的測試

### 涵蓋所有套件管理器的單元測試

這 5 項同時檢查四個套件管理器，各清單的「怎麼跑」都把它們算進去。

| 檔案 | 測試 | 驗證什麼 |
| --- | --- | --- |
| `core/src/package_manager/mod.rs` | `ids_round_trip` | 管理器代碼（`apt`、`dnf`、`pacman`、`apk`）與列舉值互相轉換；不認得的代碼與大小寫不同的寫法都不接受 |
| | `root_never_uses_sudo_and_sudo_always_answers_the_prompt_marker` | root 的命令都不含 sudo；一般帳號的命令都經 `sudo -S -p` 提示標記；重新啟動狀態、dist-upgrade 與模擬只有 APT 有 |
| `core/src/package_manager/operation.rs` | `upgrade_errors_are_failures_even_with_zero_exit` | 退出碼 0 但輸出有該管理器自己的錯誤行（APT `E:`、DNF `Error:`、pacman `error:`、apk `ERROR:`）時算失敗；別的管理器的錯誤前綴只是文字 |
| | `only_the_chosen_or_recorded_manager_may_run` | 只執行使用者選的或偵測紀錄裡的管理器；選了主機上沒有的就拒絕 |
| `core/src/package_manager/detect.rs` | `detection_names_the_distribution_and_prefers_the_native_manager` | 系統偵測讀出發行版名稱，並優先選用發行版原生的管理器 |

### 不屬於某個套件管理器的 core 整合測試

這些測試在 `core/tests/workflow.rs`，使用 APT 模擬主機，但驗證的是與套件管理器無關的流程。

| 測試 | 驗證什麼 |
| --- | --- |
| `imported_private_key_authentication_works` | 匯入的私鑰可以登入 |
| `probe_never_submits_credentials_and_changed_key_blocks_authentication` | 取得指紋時不送出認證；主機金鑰改變時拒絕認證 |
| `failed_authentication_is_reported_without_package_results` | 登入失敗時回報錯誤，不帶任何套件結果 |
| `repeated_sudo_prompt_fails_without_repeated_password_submission` | sudo 再次詢問時直接失敗，不重送密碼 |
| `root_output_printing_the_sudo_marker_gets_no_password_and_no_abort` | root 的命令輸出裡出現提示標記時，不送密碼也不中止 |
| `batch_runs_each_host_and_saves_per_host_results` | 批次逐台執行並各自保存結果 |
| `batch_reports_each_host_and_an_abort_skips_hosts_not_started` | 批次回報每台主機；中斷時跳過還沒開始的主機 |
| `a_stopped_upgrade_is_saved_as_unknown_and_audited_as_cancelled` | 被中斷的升級存成「未知」，操作日誌記為取消 |
| `a_stopped_batch_keeps_finished_hosts_and_marks_running_upgrades_unknown` | 被中斷的批次保留已完成的主機，執行中的升級記為未知 |
| `persistence_saves_check_and_resets_trust_when_endpoint_changes` | 保存檢查結果；編輯器不能改寫偵測結果；位址或埠改變時重設指紋信任 |
| `ssh_apt_results_roundtrip_inside_luksbox_and_remain_cli_compatible` | 檢查結果存進 LUKSbox 後讀回不變，CLI 仍可使用 |
| `hosts_share_a_stored_key_that_cannot_be_deleted_while_in_use` | 多台主機共用一把儲存的金鑰，使用中不能刪除 |
| `hosts_are_dragged_between_groups_and_survive_group_deletion` | 主機在群組間拖曳，刪除群組後主機仍在 |
| `an_interactive_shell_echoes_keys_follows_resizes_and_ends` | 互動 shell 回顯按鍵、跟隨視窗大小、正常結束 |
| `a_shell_stops_reading_while_the_screen_is_behind` | 畫面跟不上時 shell 暫停讀取（流量控制） |
| `hosts_saved_for_bsd_or_yum_come_back_linux_and_detected_automatically` | 舊版存成 BSD 或 YUM 的主機讀出來是 Linux，套件管理器改回自動偵測 |

只跑這些：`cargo test -p misf-core --test workflow`，再以名稱過濾（見上面的參數說明）。

### UI 測試

`app/src/ui/tests.rs` 測介面狀態與畫面操作，不分套件管理器：`cargo test -p misf`。說明見 [開發說明](../PROJECT-DEV.md#執行所有測試)。
