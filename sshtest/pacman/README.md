# sshtest／pacman

> 最後修改：2026-11-17 ｜ 基於 commit：`8aaa1b9`

對真的 Arch Linux 容器測試 pacman：sshd、sudo、pacman、pacman-contrib 的 `checkupdates` 與 fakeroot 都是真的。執行方式與共用框架見 [上一層的說明](../README.md)。

```powershell
cargo test -p misf-sshtest --test pacman -- --ignored
```

不含慢速測試約 40 秒（22 項平行執行）；加上 `MISF_SSHTEST_SLOW=1` 約 4 分鐘。基底映像以 `MISF_SSHTEST_PACMAN_BASE` 指定，預設 `archlinux:latest`。Arch 是滾動發行，映像沒有固定版本：建置時會從 Arch 的套件庫安裝當時最新的 `openssh`、`sudo`、`pacman-contrib`、`fakeroot`、`libnewt`、`procps-ng`，所以隔一段時間重建，pacman 的版本與輸出可能改變。

## 映像（`docker/`）

- `Dockerfile`：建立三個帳號與另一個只用來跑 `makepkg` 的 `builder`。`pacman.conf` 只剩本機套件庫 `[misf-test]`（`file:///opt/misf/repo`），簽章檢查關閉（測試套件沒有簽章）。
- `build-packages.sh`：以 `makepkg` 建出測試用套件（`makepkg` 不能用 root 執行）。每個套件的 1.0 版已安裝在映像裡，2.0 版放在 `/opt/misf/pool/v2`，各自以一種方式搗亂，多半寫在 `post_upgrade`。
- `misf-scenario`：把指定套件的 2.0 版發布到本機套件庫並以 `repo-add` 重建資料庫；`--v1` 發布全部 1.0 版（只在建置映像時用）。

| 套件 | 2.0 版做什麼 |
| --- | --- |
| `misf-ok` | 正常升級 |
| `misf-fail` | `post_upgrade` 回傳 1 |
| `misf-prefail` | `pre_upgrade` 回傳 1 |
| `misf-slow` | `post_upgrade` 睡 `/run/misf-slow.seconds` 秒（預設 20），期間可斷線或殺掉 pacman |
| `misf-noise` | 印出 ANSI 顏色、視窗標題、`\r` 進度、NUL、退格、非 UTF-8 位元組、十萬字元的一行、沒有結尾的跳脫序列 |
| `misf-eline` | 印出一行以 `error:` 開頭的文字後成功 |
| `misf-marker` | 印出 MISF 的 sudo 提示標記，並把收到的輸入存到 `/run/misf-marker.stdin` |
| `misf-prompt` | 印出 `[y/N]` 問題後從標準輸入 `read` |
| `misf-dialog` | 直接呼叫 `whiptail` 開對話框 |
| `misf-tui` | 強制 `TERM=xterm`，並直接在 `/dev/tty` 上畫對話框 |
| `misf-tty` | 在 `/dev/tty` 上問問題 |
| `misf-conf` | 改動管理者也改過的 `backup` 設定檔 |
| `misf-dep`、`misf-newdep` | `misf-dep` 新增對未安裝套件的相依，升級時會一併安裝 |
| `misf-broken` | 相依一個不存在的套件，無法安裝 |

## pacman 的特殊行為

寫給日後修改 core 的人；每一項都有測試對應。

- **不能只同步、不升級。** Arch 不支援 partial upgrade，所以 MISF 的檢查不執行 `pacman -Sy`，改用 `checkupdates`：它在 `/tmp/checkup-db-<uid>` 同步一份私有資料庫，不需要 root，也不需要 pacman 的鎖。測試確認檢查後系統自己的資料庫仍不知道有更新（`pacman -Qu` 沒有輸出）。
- **檢查需要另外兩個套件。** 沒有 `pacman-contrib` 或 `fakeroot` 的主機無法檢查，連升級也不會開始（升級前一定先檢查）。
- **腳本失敗時 pacman 仍以 0 結束。** `pre_upgrade`／`post_upgrade` 失敗只印一行 `error: command failed to execute correctly`，套件照樣安裝（連 `pre_upgrade` 失敗也是）。MISF 靠那一行判定失敗。
- **`--noconfirm` 是選預設答案，不是全部答「是」。** 相依不滿足時 pacman 問「要不要略過這個套件」，預設是否，整個升級因此取消；套件檔損毀時問「要不要刪除」，預設是。
- **鎖檔是一般檔案，程序死了不會自己消失。** `/var/lib/pacman/db.lck` 只在 pacman 正常結束時刪除；另一個 pacman 佔著鎖時，後來的不會等，直接失敗（見 GAP 3）。
- **清單沒有架構與套件庫。** `checkupdates` 只印 `名稱 舊版 -> 新版`，結果裡這兩欄是空的。
- **腳本沒有標準輸入**（與 DNF 相同）：`read` 立刻讀到結尾；MISF 送出的密碼到不了套件腳本。
- **腳本輸出原樣印出，沒有前綴。** 所以腳本印的 `error:` 行與 pacman 自己的分不出來（見 GAP 5）。一行超過約 2 KiB 會被切成多行；一行裡 NUL 之後的內容（含換行）會被丟掉。
- **新設定檔存成 `.pacnew`**，管理者改過的檔案不動，不會詢問。

## 測試項目

| 檔案 | 測試 | 情境 |
| --- | --- | --- |
| `upgrade.rs` | `a_check_leaves_the_system_databases_alone_…` | 基準：檢查不用 sudo、不動系統資料庫；升級、連帶安裝相依、`.pacnew` |
| | `root_upgrades_without_sudo_…` | root 不經 sudo、不用 PTY；沒有 dist-upgrade |
| | `passwordless_sudo_upgrades_…` | 免密碼 sudo、不用 PTY |
| `failures.rs` | `a_failing_script_fails_the_run_…` | `post_upgrade`／`pre_upgrade` 失敗 |
| | `one_package_that_cannot_be_installed_…` | 一個套件相依不滿足 |
| | `a_broken_repository_…` | 套件庫資料庫抓不到 |
| | `a_host_without_checkupdates_or_fakeroot_…` | 缺 fakeroot、缺 checkupdates |
| | `a_package_file_that_cannot_be_read_…` | 套件檔不見、套件檔被截斷 |
| | `pacman_killed_half_way_…` | 升級途中 pacman 被殺掉，留下鎖檔 |
| | `an_upgrade_while_another_pacman_holds_the_lock_…` | 另一個 pacman 佔著鎖 |
| `output.rs` | `garbage_from_a_package_script_…` | 不正常的輸出 |
| | `a_script_line_that_looks_like_a_pacman_error_…` | 腳本印出 `error:` 行 |
| | `root_ignores_…`、`the_marker_after_sudo_…`、`with_passwordless_sudo_…` | 輸出裡出現 sudo 提示標記 |
| `prompts.rs` | `a_dialog_cannot_open_on_the_dumb_terminal_…` | 對話框在 `TERM=dumb` 下開不起來 |
| | `a_question_on_the_scripts_input_…` | 腳本從標準輸入等回答 |
| | `a_dialog_drawn_on_the_terminal_…` | TUI 畫面真的出現 |
| | `a_question_asked_on_the_terminal_…` | 腳本在終端機上等回答 |
| `connection.rs` | `a_cut_connection_with_a_pty_…` | root 有 PTY 時斷線 |
| | `a_cut_connection_without_a_pty_…` | 沒有 PTY 時斷線，留下鎖檔 |
| | `a_cut_connection_under_sudo_with_a_pty_…` | 經 sudo 且有 PTY 時斷線 |
| | `a_silently_dead_link_…`（慢） | 網路無聲中斷 |

`support.rs` 放 pacman 專用的輔助：`start`、`Pacman`（已安裝的版本、pacman 是否還在執行、鎖檔是否存在）。

## 測試發現的缺口

測試的斷言寫的是 MISF **目前**的行為。下列項目不是期望的行為，在測試裡以 `GAP n` 標示（編號只在 pacman 內有效）；日後修正 core 時要一併修改對應的斷言。

| 編號 | 現象 | 原因 |
| --- | --- | --- |
| GAP 1 | 套件庫抓不到時，輸出只有 `==> ERROR: Cannot fetch updates`，看不出是哪個套件庫、什麼原因；錯誤訊息還會附上「需要安裝 pacman-contrib 與 fakeroot」，即使兩者都裝了。 | MISF 把 `checkupdates` 同步時的輸出導到 `/dev/null`；只要索引更新失敗就附上那句提示，不分原因。 |
| GAP 2 | pacman 被訊號殺掉時，錯誤只有「遠端命令被訊號中止」，沒提升級被打斷，升級步驟已收到的輸出也沒有保存。 | `ssh::execute` 收到 exit-signal 就直接回傳錯誤。 |
| GAP 3 | **過期的鎖檔。** pacman 被殺掉，或在沒有 PTY 時連線中斷（包括 MISF 的「中斷」與 GAP 6），`/var/lib/pacman/db.lck` 會留下。當下主機檢查正常；等下一次有更新時，升級一律失敗（`unable to lock database`），直到有人手動刪除該檔。 | 沒有 PTY 時 pacman 在套件腳本跑完後，往已關閉的連線寫輸出而被結束，來不及刪鎖檔。MISF 沒有檢查鎖檔，也沒有在錯誤訊息裡提示。 |
| GAP 4 | 套件腳本印出沒有結尾的跳脫序列後，該命令之後的輸出在保存的紀錄裡全部消失。即時終端機只少那一行。 | `transcript::readable_output` 把 OSC 字串一路吃到結尾符號。 |
| GAP 5 | 套件腳本印出以 `error:` 開頭的一行，pacman 退出碼 0、套件已裝好，結果卻存成「套件升級失敗（退出碼 0）」。 | pacman 把腳本輸出原樣印出；因為腳本失敗時退出碼也是 0，MISF 只能靠這種行判斷，兩者分不開。 |
| GAP 6 | 需要密碼的 sudo：sudo 已收下密碼、pacman 正在升級時，套件輸出出現提示標記，MISF 回報「sudo 密碼錯誤或需要額外驗證」並斷線，升級步驟的輸出沒有保存。套件其實裝好了。 | `ssh::execute` 把第二次出現的標記當成 sudo 再次詢問。 |
| GAP 7 | 有 PTY 時，套件腳本自己去 `/dev/tty` 問問題或畫對話框，MISF 一直等到 30 分鐘上限。中斷後 pacman 會結束，鎖檔也會刪除。 | 沒有偵測長時間沒有輸出，也沒有辦法回答。 |
| GAP 8 | 升級途中連線中斷：錯誤只有「SSH 連線結束但沒有命令退出碼」，沒有提到升級可能仍在進行，升級步驟的輸出也沒有保存。pacman 會把套件裝完。無聲斷線約 210 秒才放棄。 | 與 APT、DNF 相同。 |
| GAP 9 | 經 sudo 且有 PTY 時斷線：正在執行的套件腳本被中途結束，但套件仍算已安裝，沒有任何紀錄指出那個腳本沒跑完。 | sudo 把掛斷訊號傳給腳本；pacman 不把腳本的結果當成安裝失敗。 |

另外，升級被中斷後 MISF 的提示寫的是 `sudo dpkg --configure -a`，對 pacman 主機不適用（`8aaa1b9` 的驗證紀錄已記載，沒有對應的測試）。

測試也確認了這些保護有效：檢查完全不經 sudo、不執行 `pacman -Sy`；`backup` 設定檔保留並產生 `.pacnew`；`TERM=dumb` 讓 `whiptail` 直接失敗；資料庫抓不到、套件檔不見或損毀、相依不滿足、鎖被佔用都存成失敗且不動到套件；不正常的輸出不會讓結果出錯或把密碼帶進輸出。

## 沒有涵蓋的範圍

- 真實的 Arch 主機（有 systemd、有 pacman hooks 的完整系統）；容器裡只有一個 hook。
- AUR、`IgnorePkg`（解析有單元測試，容器測試沒有）、套件簽章與金鑰環（`SigLevel = Never`）、`.pacsave`、套件互相衝突或取代（`replaces`／`conflicts`）時的提問。
- Manjaro 等衍生發行版。
- 套件庫是本機 `file:` 來源，沒有測 HTTP 鏡像站變慢、逾時。
- 只有密碼登入，沒有測私鑰登入與批次作業。
