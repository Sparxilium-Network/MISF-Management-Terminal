# sshtest／DNF

> 最後修改：2026-11-17 ｜ 基於 commit：`ffff655`

對真的 Enterprise Linux 容器測試 DNF：sshd、sudo、dnf 4 與 rpm 都是真的。執行方式與共用框架見 [上一層的說明](../README.md)。

```powershell
cargo test -p misf-sshtest --test dnf -- --ignored
```

不含慢速測試約 45 秒（21 項平行執行）；加上 `MISF_SSHTEST_SLOW=1` 約 4 分鐘。基底映像以 `MISF_SSHTEST_DNF_BASE` 指定，預設 `rockylinux/rockylinux:9`。

| 基底映像 | 結果（2026-11-17） |
| --- | --- |
| `rockylinux/rockylinux:9`（Rocky Linux 9.8，dnf 4.14） | 22 項通過 |
| `almalinux:9` | 22 項通過 |
| `fedora:latest`（dnf5 5.4） | **不能用**：MISF 讀不懂 dnf5 的清單（見 GAP 11），每個測試都在第一次檢查就失敗；映像本身也還留著 Fedora 自己的套件庫快取 |

## 映像（`docker/`）

- `Dockerfile`：安裝 `openssh-server`、`sudo`、`newt`（whiptail）、`procps-ng`、`rpm-build`、`createrepo_c`，建立三個帳號。套件庫只剩 `file:///opt/misf/repo`（代碼 `misf-test`，`gpgcheck=0`）。
- `build-packages.sh`：以 `rpmbuild` 建出測試用套件。每個套件的 1.0 版已安裝在映像裡，2.0 版放在 `/opt/misf/pool/v2`，各自以一種方式搗亂。腳本內容會經過 rpm 的巨集展開，不能寫百分比符號。
- `misf-scenario`：把指定套件的 2.0 版發布到本機套件庫並重建中繼資料。

| 套件 | 2.0 版做什麼 |
| --- | --- |
| `misf-ok` | 正常升級 |
| `misf-fail` | `%post` 回傳 1 |
| `misf-prefail` | `%pre` 回傳 1，rpm 因此拒絕安裝它 |
| `misf-slow` | `%post` 睡 `/run/misf-slow.seconds` 秒（預設 20），期間可斷線或殺掉 dnf |
| `misf-noise` | 印出 ANSI 顏色、視窗標題、`\r` 進度、非 UTF-8 位元組、NUL、退格、十萬字元的一行、沒有結尾的跳脫序列 |
| `misf-eline` | 印出一行以 `Error:` 開頭的文字後成功 |
| `misf-marker` | 印出 MISF 的 sudo 提示標記，並把收到的輸入存到 `/run/misf-marker.stdin` |
| `misf-dialog` | 直接呼叫 `whiptail` 開對話框 |
| `misf-tui` | 強制 `TERM=xterm`，並直接在 `/dev/tty` 上畫對話框 |
| `misf-prompt` | 印出 `[y/N]` 問題後從標準輸入 `read` |
| `misf-tty` | 同樣的問題，但在 `/dev/tty` 上問 |
| `misf-conf` | 改動管理者也改過的 `%config(noreplace)` 設定檔 |
| `misf-dep`、`misf-newdep` | `misf-dep` 新增對未安裝套件的相依，升級時會一併安裝 |
| `misf-broken` | 相依一個不存在的套件，無法安裝 |

## 測試項目

| 檔案 | 測試 | 情境 |
| --- | --- | --- |
| `upgrade.rs` | `a_clean_upgrade_through_sudo_…` | 基準：檢查、升級、連帶安裝相依、設定檔保留並產生 `.rpmnew` |
| | `root_upgrades_without_sudo_…` | root 不經 sudo、不用 PTY；DNF 沒有 dist-upgrade |
| `failures.rs` | `a_failing_post_script_…` | `%post` 失敗 |
| | `a_failing_pre_script_…` | `%pre` 失敗，交易做到一半 |
| | `one_package_that_cannot_be_installed_…` | 一個套件相依不滿足 |
| | `a_broken_repository_…` | 套件庫中繼資料抓不到 |
| | `a_repository_dnf_may_skip_…` | 抓不到但設了 `skip_if_unavailable` 的套件庫 |
| | `a_package_file_that_cannot_be_read_…` | 套件檔不見、套件檔被截斷 |
| | `dnf_killed_half_way_…` | 升級途中 dnf 被殺掉 |
| | `an_upgrade_waits_for_another_dnf_…` | 另一個 dnf 佔著鎖 |
| `output.rs` | `garbage_from_a_package_script_…` | 不正常的輸出 |
| | `a_script_line_that_looks_like_a_dnf_error_…` | 腳本印出 `Error:` 行 |
| | `root_ignores_…`、`the_marker_after_sudo_…`、`with_passwordless_sudo_…` | 輸出裡出現 sudo 提示標記 |
| `prompts.rs` | `a_dialog_cannot_open_on_the_dumb_terminal_…` | 對話框在 `TERM=dumb` 下開不起來 |
| | `a_question_on_the_scripts_input_…` | 腳本從標準輸入等回答 |
| | `a_dialog_drawn_on_the_terminal_…` | TUI 畫面真的出現 |
| | `a_question_asked_on_the_terminal_…` | 腳本在終端機上等回答 |
| `connection.rs` | `a_cut_connection_fails_the_run_while_root_s_dnf_…` | root 升級途中斷線 |
| | `a_cut_connection_under_sudo_with_a_pty_…` | 經 sudo 且有 PTY 時斷線 |
| | `a_silently_dead_link_…`（慢） | 網路無聲中斷 |

`support.rs` 放 DNF 專用的輔助：`start`、`Rpm`（已安裝的版本、dnf 是否還在執行）。

## 測試發現的缺口

測試的斷言寫的是 MISF **目前**的行為。下列項目不是期望的行為，在測試裡以 `GAP n` 標示（編號只在 DNF 內有效，與 APT 的編號無關）；日後修正 core 時要一併修改對應的斷言。

| 編號 | 現象 | 原因 |
| --- | --- | --- |
| GAP 1 | 套件的 `%post` 失敗（包括對話框開不起來），結果仍存成成功，只有輸出裡有一行 `Error in POSTIN scriptlet`。 | rpm 把 `%post` 失敗當成警告，dnf 退出碼 0；MISF 只認 `Error:` 開頭的行。 |
| GAP 2 | 設了 `skip_if_unavailable` 的套件庫抓不到時，檢查照樣成功，完全沒提到少了一個套件庫。 | `dnf -q makecache` 對被略過的套件庫不印任何東西，`rpm::update_failed` 要找的 `Ignoring repositories` 不會出現在這一步。 |
| GAP 3 | 輸出裡有 `Error:` 開頭的行，dnf 退出碼 0、套件已裝好，結果卻存成「套件升級失敗（退出碼 0）」。套件腳本印出這種行會這樣；GAP 2 的套件庫在升級時印出的 `Error: Failed to download metadata…` 也會。 | `validate_upgrade` 把任何 `E:`／`Error:` 開頭的行都當成錯誤。 |
| GAP 4 | dnf 被訊號殺掉時，錯誤只有「遠端命令被訊號中止」，沒提升級被打斷，升級步驟已收到的輸出也沒有保存。 | `ssh::execute` 收到 exit-signal 就直接回傳錯誤。 |
| GAP 5 | GAP 4 之後，同一個套件的新舊兩版同時裝著（`dnf check` 會報錯）。再升級會把剩下的套件裝完並顯示成功，重複的套件一直留著，沒有任何提示。 | 重複安裝的套件不在 `check-update` 清單裡，MISF 也沒有檢查 rpm 資料庫的狀態。 |
| GAP 6 | 套件腳本印出沒有結尾的跳脫序列後，該命令之後的輸出（包括 dnf 的 `Complete!`）在保存的紀錄裡全部消失。即時終端機只少那一行。 | `transcript::readable_output` 把 OSC 字串一路吃到結尾符號。 |
| GAP 7 | 需要密碼的 sudo：sudo 已收下密碼、dnf 正在升級時，套件輸出出現提示標記，MISF 回報「sudo 密碼錯誤或需要額外驗證」並斷線，升級步驟的輸出沒有保存。套件其實裝好了。 | `ssh::execute` 把第二次出現的標記當成 sudo 再次詢問。 |
| GAP 8 | 有 PTY 時，套件腳本自己去 `/dev/tty` 問問題或畫對話框，MISF 一直等到 30 分鐘上限。 | 沒有偵測長時間沒有輸出，也沒有辦法回答。 |
| GAP 9 | 升級途中連線中斷：錯誤只有「SSH 連線結束但沒有命令退出碼」，沒有提到升級可能仍在進行，升級步驟的輸出也沒有保存。root 的 dnf 不受影響，會把升級做完。無聲斷線約 210 秒才放棄。 | 與 APT 的 GAP 8 相同。 |
| GAP 10 | 經 sudo 且有 PTY 時斷線：正在執行的套件腳本被中途結束，但 rpm 仍把套件算成已安裝，dnf 把交易做完；之後檢查顯示沒有待更新套件，沒有任何紀錄指出那個腳本沒跑完。 | sudo 把掛斷訊號傳給腳本；rpm 不把 `%post` 的結果當成安裝失敗。 |
| GAP 11 | Fedora（dnf5）主機完全不能檢查：「無法解析 DNF／YUM 套件清單」。 | dnf5 的 `check-update` 會先印一行 `Upgrades` 標題，`rpm::parse_pending` 把它當成套件欄位；`dnf5 -q makecache` 也仍會印出 `Metadata cache created.`。這一項沒有對應的測試，是在試 Fedora 基底映像時發現的。 |

與 APT 不同、而且是好事的地方：

- rpm 不給套件腳本標準輸入：腳本裡的 `read` 立刻讀到結尾，不會卡住；免密碼 sudo 時 MISF 送出的密碼也到不了套件腳本。
- `%pre` 失敗的套件會留在待更新清單裡，再檢查看得到（APT 的 half-configured 套件看不到）。
- 另一個 dnf 佔著鎖時，dnf 會排隊等它結束再繼續，不會失敗。
- 被對話框卡住後中斷，dnf 會結束、交易也會完成，不會留下卡住的程序。

測試也確認了這些保護有效：`%config(noreplace)` 保留管理者改過的設定檔；`TERM=dumb` 讓 `whiptail` 直接失敗；中繼資料抓不到、套件檔不見或被截斷、`%pre` 失敗、相依不滿足都存成失敗且不動到套件；不正常的輸出不會讓結果出錯或把密碼帶進輸出。

## 沒有涵蓋的範圍

- Fedora／dnf5（見上表與 GAP 11）、RHEL 8、CentOS Stream。
- 套件庫是本機 `file:` 來源，沒有測 HTTP 鏡像站變慢、逾時，也沒有測 GPG 簽章錯誤（`gpgcheck=0`）。
- 沒有測模組（modularity）、`dnf-automatic`、需要重新開機的判斷（MISF 目前只對 APT 讀重啟狀態）。
- Enterprise Linux 預設 `best=True`；沒有測 `best=False`（Fedora 的預設）下略過套件的行為。
- 只有密碼登入，沒有測私鑰登入與批次作業。
