# sshtest／APT

> 最後修改：2026-11-17 ｜ 基於 commit：`8aaa1b9`

對真的 Debian／Ubuntu 容器測試 APT：sshd、sudo、apt、dpkg、debconf 與 needrestart 都是真的。執行方式與共用框架見 [上一層的說明](../README.md)。

```powershell
cargo test -p misf-sshtest --test apt -- --ignored
```

不含慢速測試約 45 秒（17 項平行執行）；加上 `MISF_SSHTEST_SLOW=1` 約 4 分鐘。基底映像以 `MISF_SSHTEST_APT_BASE` 指定，預設 `debian:13`。

## 映像（`docker/`）

- `Dockerfile`：安裝 `openssh-server`、`sudo`、`whiptail`、`apt-utils`、`procps`、`needrestart`，建立三個帳號。APT 來源只剩 `file:/opt/misf/repo`。
- `build-packages.sh`：建出測試用套件。每個套件的 1.0 版已安裝在映像裡，2.0 版放在 `/opt/misf/pool/v2`，各自以一種方式搗亂。
- `misf-scenario`：把指定套件的 2.0 版發布到本機套件庫（測試裡 `start(帳號, 是否使用 PTY, &["套件"])` 的第三個參數）。

| 套件 | 2.0 版做什麼 |
| --- | --- |
| `misf-ok` | 正常升級，並建立 `/var/run/reboot-required` |
| `misf-fail` | postinst 回傳 1 |
| `misf-slow` | postinst 睡 `/run/misf-slow.seconds` 秒（預設 20），期間可斷線或殺掉 dpkg |
| `misf-noise` | 印出 ANSI 顏色、視窗標題、`\r` 進度、非 UTF-8 位元組、NUL、退格、十萬字元的一行、沒有結尾的跳脫序列 |
| `misf-eline` | 印出一行以 `E:` 開頭的文字後成功 |
| `misf-marker` | 印出 MISF 的 sudo 提示標記，並把收到的輸入存到 `/run/misf-marker.stdin` |
| `misf-dialog` | 直接呼叫 `whiptail` 開對話框 |
| `misf-tui` | 同上，但強制 `TERM=xterm`，對話框真的會畫出來 |
| `misf-prompt` | 印出 `[y/N]` 問題後 `read` 等待回答 |
| `misf-debconf` | 以 critical 優先度問 debconf 問題 |
| `misf-conf` | 改動管理者也改過的 conffile |
| `misf-kept`、`misf-newdep` | `misf-kept` 新增對未安裝套件的相依，`upgrade` 會把它 kept back |

新增情境：在 `build-packages.sh` 加一個套件（`start`／`script`／`finish`），把名稱加進安裝 1.0 版的迴圈，再在對應主題的檔案寫測試。

## 測試項目

| 檔案 | 測試 | 情境 |
| --- | --- | --- |
| `upgrade.rs` | `a_clean_upgrade_through_sudo_…` | 基準：檢查、升級、真實 needrestart 輸出、reboot-required |
| | `questions_apt_would_ask_…` | debconf 問題與 conffile 衝突都不會跳出來 |
| | `kept_back_packages_…` | kept back、dist-upgrade 模擬計畫、dist-upgrade |
| `failures.rs` | `a_failing_package_script_…` | 升級到一半失敗，一個套件完成、一個停在 half-configured |
| | `a_broken_repository_…` | 套件庫索引抓不到 |
| | `a_package_file_that_cannot_be_fetched_…` | 套件檔不見、套件檔被截斷 |
| | `dpkg_killed_half_way_…` | 升級途中 dpkg 被殺掉 |
| | `an_upgrade_while_another_apt_holds_the_lock_…` | 另一個 apt 佔著鎖 |
| `output.rs` | `garbage_from_a_package_script_…` | 不正常的輸出 |
| | `a_script_line_that_looks_like_an_apt_error_…` | 腳本印出 `E:` 行 |
| | `root_ignores_…`、`the_marker_after_sudo_…`、`with_passwordless_sudo_…` | 輸出裡出現 sudo 提示標記（root、要密碼的 sudo、免密碼 sudo） |
| `prompts.rs` | `a_dialog_cannot_open_on_the_dumb_terminal_…` | 對話框在 `TERM=dumb` 下開不起來 |
| | `a_drawn_dialog_keeps_the_upgrade_waiting_…` | TUI 畫面真的出現 |
| | `a_script_waiting_for_an_answer_…` | 腳本等待鍵盤回答 |
| `connection.rs` | `a_cut_connection_…` | 升級途中斷線 |
| | `a_silently_dead_link_…`（慢） | 網路無聲中斷，沒有人關閉連線 |

`support.rs` 放 APT 專用的輔助：`start`、`Dpkg`（`dpkg` 狀態、`dpkg --audit`、apt 是否還在執行）、`assert_plain`、`assert_failed`。

## 測試發現的缺口

測試的斷言寫的是 MISF **目前**的行為。下列項目不是期望的行為，在測試裡以 `GAP n` 標示；日後修正 core 時要一併修改對應的斷言。2026-11-16 在 `debian:13` 與 `ubuntu:26.04` 上結果相同。

| 編號 | 現象 | 原因 |
| --- | --- | --- |
| GAP 1 | 升級半途失敗（postinst 失敗、dpkg 被殺、被對話框卡住）之後，再檢查或再升級都顯示「成功、沒有待更新套件」，主機看起來是最新的，但 `dpkg --audit` 仍回報 half-configured。 | 已解開但未設定完成的套件不再列在 `apt list --upgradable`；清單是空的，MISF 就不執行 `apt-get upgrade`，看不到 APT 的 `dpkg was interrupted` 錯誤，也沒有另外檢查 dpkg 狀態。 |
| GAP 2 | 套件腳本印出沒有結尾的跳脫序列（`ESC ]`…）後，該命令之後的輸出在保存的紀錄裡全部消失。即時終端機只少那一行。 | `transcript::readable_output` 把 OSC 字串一路吃到結尾符號，整段命令輸出一次處理；即時輸出是逐行處理。 |
| GAP 3 | 套件腳本印出以 `E:` 開頭的一行，APT 退出碼 0、套件已裝好，結果卻存成「套件升級失敗（退出碼 0）」。 | APT 主機上，`validate_upgrade` 把任何 `E:` 開頭的行都當成 APT 錯誤（`8aaa1b9` 起各管理器只認自己的錯誤前綴）。 |
| GAP 4 | 需要密碼的 sudo：sudo 已收下密碼、APT 正在升級時，套件輸出出現提示標記，MISF 回報「sudo 密碼錯誤或需要額外驗證」並斷線，升級步驟的輸出沒有保存。遠端的升級照常完成。 | `ssh::execute` 把第二次出現的標記當成 sudo 再次詢問。 |
| GAP 5 | 免密碼（`NOPASSWD`）sudo：套件輸出出現提示標記時，SSH 密碼被送進套件腳本的標準輸入（有無 PTY 都一樣）。保存的輸出與即時輸出都沒有密碼。 | sudo 沒有詢問，第一個標記就被當成 sudo 的提示。使用指南已記載免密碼 sudo 的風險。 |
| GAP 6 | 套件腳本等待輸入（`read`，或強制 `TERM=xterm` 畫出的對話框）時，MISF 一直等到 30 分鐘上限。對話框的文字會出現在即時終端機，但沒有辦法按鍵。 | MISF 不對命令送出輸入結束（EOF），也沒有偵測長時間沒有輸出。APT 另外給 dpkg 一個自己的終端機，所以沒開 PTY 也一樣。 |
| GAP 7 | 被 GAP 6 卡住後中斷（或逾時）：遠端的 `apt-get` 仍在執行並持有 dpkg 鎖，對話框繼續等待。沒開 PTY 而腳本用 `read` 時例外：連線一斷，腳本讀到空白回答，升級自己完成。 | 中斷只關閉 SSH 連線，不處理遠端程序。 |
| GAP 8 | 升級途中連線中斷：錯誤只有「SSH 連線結束但沒有命令退出碼」，沒有提到升級可能仍在進行，升級步驟已收到的輸出也沒有保存。遠端的 `apt-get` 不受影響，有無 PTY 都會把升級做完。網路無聲中斷時，MISF 約 210 秒後才放棄（6 次間隔 30 秒的 keepalive）。 | `ssh::execute` 在連線結束時直接回傳錯誤，`Run::install` 還沒把輸出寫進紀錄。 |

測試也確認了這些保護有效：

- `DEBIAN_FRONTEND=noninteractive` 經過 `sudo -- env` 仍然生效，debconf 的 critical 問題採用預設值。
- `--force-confold` 保留管理者改過的設定檔，新版存成 `.dpkg-dist`，不會詢問。
- MISF 要求的終端機類型是 `dumb`，`whiptail` 這類對話框會直接失敗，不會卡住。
- 索引更新失敗、套件檔抓不到或雜湊不符、postinst 失敗、dpkg 被殺、鎖被佔用，都存成失敗，且不會把沒做的事算成已升級。
- 控制字元、ANSI 序列、非 UTF-8 位元組、十萬字元的長行都不會讓結果出錯或把密碼帶進輸出。
- 真實的 `needrestart -r l -b` 輸出與 `/var/run/reboot-required` 解析正確。

## 沒有涵蓋的範圍

- 容器裡沒有 systemd，也不能真的重新開機；`needrestart` 讀不到可用的核心版本（`KSTA: 0`），服務重啟清單一直是空的。
- 套件庫是本機 `file:` 來源，沒有測 HTTP 鏡像站變慢、逾時或簽章錯誤。
- 映像裝的是 `sudo` 套件（Debian 13 為 1.9.16p2，Ubuntu 26.04 為 1.9.17p2），沒有測 sudo-rs。
- 沒有測 phased updates（需要 Ubuntu 套件庫的 `Phased-Update-Percentage`）。
- 只有密碼登入，沒有測私鑰登入與批次作業。
