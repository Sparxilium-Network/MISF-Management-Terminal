# sshtest／apk

> 最後修改：2026-11-17 ｜ 基於 commit：`8aaa1b9`

對真的 Alpine 容器測試 apk：sshd、sudo 與 apk-tools 都是真的。執行方式與共用框架見 [上一層的說明](../README.md)。

```powershell
cargo test -p misf-sshtest --test apk -- --ignored
```

不含慢速測試約 70 秒（21 項平行執行）；加上 `MISF_SSHTEST_SLOW=1` 約 4 分鐘。基底映像以 `MISF_SSHTEST_APK_BASE` 指定，預設 `alpine:3.24`（Alpine 3.24.4，apk-tools 3.0.9）。

映像與大部分測試的草稿來自實作 core 的 apk 支援時（`8aaa1b9`）；這裡補上終端機上的對話框與問題、有 PTY 時斷線、套件檔讀不到、無聲斷線，以及說明文件。

## 映像（`docker/`）

- `Dockerfile`：安裝 `openssh-server`、`sudo`、`abuild`、`newt`（whiptail）、`procps`，建立三個帳號與另一個只用來跑 `abuild` 的 `builder`。`/etc/apk/repositories` 只剩本機套件庫 `/opt/misf/repo`；索引以 `builder` 的金鑰簽章，公鑰放在 `/etc/apk/keys`。
- `build-packages.sh`：以 `abuild` 建出測試用套件（`abuild` 不能用 root 執行）。每個套件的 1.0 版已安裝在映像裡，2.0 版放在 `/opt/misf/pool/v2`，各自以一種方式搗亂，多半寫在 `post-upgrade` 腳本。
- `misf-scenario`：把指定套件的 2.0 版發布到本機套件庫，重建並簽署 `APKINDEX.tar.gz`；`--v1` 發布全部 1.0 版（只在建置映像時用）。

| 套件 | 2.0 版做什麼 |
| --- | --- |
| `misf-ok` | 正常升級 |
| `misf-fail` | `post-upgrade` 以 1 結束 |
| `misf-prefail` | `pre-upgrade` 以 1 結束 |
| `misf-slow` | `post-upgrade` 睡 `/run/misf-slow.seconds` 秒（預設 20），期間可斷線或殺掉 apk |
| `misf-noise` | 印出 ANSI 顏色、視窗標題、`\r` 進度、NUL、退格、非 UTF-8 位元組、沒有結尾的跳脫序列（沒有超長行：apk 以固定大小的緩衝區逐行讀腳本輸出，太長的行會讓腳本的管線斷掉） |
| `misf-eline` | 印出一行以 `ERROR:` 開頭的文字後成功 |
| `misf-marker` | 印出 MISF 的 sudo 提示標記，並把收到的輸入存到 `/run/misf-marker.stdin` |
| `misf-prompt` | 印出 `[y/N]` 問題後從標準輸入 `read` |
| `misf-dialog` | 直接呼叫 `whiptail` 開對話框 |
| `misf-tui` | 強制 `TERM=xterm`，並直接在 `/dev/tty` 上畫對話框 |
| `misf-tty` | 在 `/dev/tty` 上問問題 |
| `misf-conf` | 改動管理者也改過的設定檔 |
| `misf-dep`、`misf-newdep` | `misf-dep` 新增對未安裝套件的相依，升級時會一併安裝 |
| `misf-broken` | 相依一個不存在的套件，無法安裝 |

## apk 的特殊行為

寫給日後修改 core 的人；每一項都有測試對應。

- **沒有 sudo。** Alpine 預設是 doas，doas 不能從標準輸入讀密碼，所以 root 以外的帳號必須另外安裝 sudo（映像已安裝）。
- **`apk update` 幾乎不會以非 0 結束。** 索引打不開或簽章不受信任都只是 `WARNING:`，退出碼仍是 0；MISF 靠警告行判定失敗。
- **連檢查都要鎖。** 另一個 apk 在執行時，`apk update` 立刻以 99 結束（`Unable to lock database`），不會等；所以這時候檢查與升級都失敗。鎖隨程序結束而釋放，不會留下過期的鎖檔。
- **最後才記帳。** apk 在整個升級結束時才寫入已安裝資料庫。途中被殺掉或被掛斷，這一輪就全部不算：套件仍是舊版、仍列為待更新，下次升級整批重做，已經跑過的套件腳本會再跑一次（見 GAP 3）。
- **邊抓邊裝。** apk 輪到哪個套件才讀它的檔案；某個套件檔讀不到時，排在它前面的已經裝好了（APT、DNF、pacman 都是先全部抓完才開始）。
- **腳本失敗不擋安裝。** `pre-upgrade`／`post-upgrade` 失敗時 apk 印出 `ERROR:` 並以 1 結束，但套件照樣安裝（連 `pre-upgrade` 失敗也是）。
- **腳本輸出有前綴。** 腳本印的每一行前面會加上 `  * `，所以腳本印的 `ERROR:` 不會被當成 apk 自己的錯誤（APT、DNF、pacman 都分不出來）。一行裡 NUL 之後的內容會被丟掉。
- **腳本沒有標準輸入，環境變數也被清掉。** `read` 立刻讀到結尾；即使有 PTY，腳本裡也沒有 `TERM`。
- **一個裝不了，全部不裝。** 相依不滿足時 apk 拒絕整個升級。
- **新設定檔存成 `.apk-new`**，管理者改過的檔案不動，不會詢問。
- **清單沒有套件庫欄位。** `apk list --upgradable` 的一行是 `名稱-版本 架構 {來源} (授權) [upgradable from: 名稱-版本]`。

## 測試項目

| 檔案 | 測試 | 情境 |
| --- | --- | --- |
| `upgrade.rs` | `a_clean_upgrade_through_sudo_…` | 基準：檢查、升級、連帶安裝相依、`.apk-new` |
| | `root_upgrades_without_sudo_…` | root 不經 sudo、不用 PTY；沒有 dist-upgrade |
| | `passwordless_sudo_lists_without_sudo_…` | 免密碼 sudo：索引更新經 sudo，清單不經 sudo |
| `failures.rs` | `a_failing_script_fails_the_run_…` | `post-upgrade`／`pre-upgrade` 失敗 |
| | `one_package_that_cannot_be_installed_…` | 一個套件相依不滿足 |
| | `a_missing_index_…` | 套件庫索引打不開（退出碼 0） |
| | `an_index_without_a_trusted_signature_…` | 索引沒有受信任的簽章 |
| | `a_package_file_that_cannot_be_read_…` | 套件檔不見、套件檔被截斷 |
| | `apk_killed_half_way_…` | 升級途中 apk 被殺掉 |
| | `while_another_apk_holds_the_lock_…` | 另一個 apk 佔著鎖 |
| `output.rs` | `garbage_from_a_package_script_…` | 不正常的輸出 |
| | `a_script_line_that_looks_like_an_apk_error_…` | 腳本印出 `ERROR:` 行（不會誤判） |
| | `root_ignores_…`、`the_marker_after_sudo_…`、`with_passwordless_sudo_…` | 輸出裡出現 sudo 提示標記 |
| | `a_question_on_the_scripts_input_…` | 腳本從標準輸入等回答 |
| `prompts.rs` | `a_dialog_cannot_open_without_a_terminal_type_…` | 對話框開不起來 |
| | `a_dialog_drawn_on_the_terminal_…` | TUI 畫面真的出現 |
| | `a_question_asked_on_the_terminal_…` | 腳本在終端機上等回答 |
| `connection.rs` | `a_cut_connection_fails_the_run_while_root_s_apk_…` | 沒有 PTY 時斷線 |
| | `a_cut_connection_with_a_pty_…` | 有 PTY 時斷線 |
| | `a_silently_dead_link_…`（慢） | 網路無聲中斷 |

`support.rs` 放 apk 專用的輔助：`start`、`Apk`（已安裝的版本、等 apk 結束）。

## 測試發現的缺口

測試的斷言寫的是 MISF **目前**的行為。下列項目不是期望的行為，在測試裡以 `GAP n` 標示（編號只在 apk 內有效）；日後修正 core 時要一併修改對應的斷言。

| 編號 | 現象 | 原因 |
| --- | --- | --- |
| GAP 1 | 需要密碼的 sudo：sudo 已收下密碼、apk 正在升級時，套件輸出出現提示標記，MISF 回報「sudo 密碼錯誤或需要額外驗證」並斷線，升級步驟的輸出沒有保存。 | `ssh::execute` 把第二次出現的標記當成 sudo 再次詢問。 |
| GAP 2 | 套件腳本印出沒有結尾的跳脫序列後，該命令之後的輸出（包括 apk 最後的 `OK:` 行）在保存的紀錄裡全部消失。即時終端機只少那一行。 | `transcript::readable_output` 把 OSC 字串一路吃到結尾符號。 |
| GAP 3 | 有 PTY 時連線中斷（包括 MISF 的「中斷」）：apk 隨即結束，這一輪什麼都沒記下。套件會再列為待更新，下次升級把已經跑過的腳本再跑一次；沒有任何紀錄指出哪些腳本其實執行過。 | apk 最後才寫資料庫；MISF 的錯誤訊息只說連線結束。 |
| GAP 4 | 有 PTY 時，套件腳本自己去 `/dev/tty` 問問題或畫對話框，MISF 一直等到 30 分鐘上限。中斷後依 GAP 3，套件仍待更新，下次升級又會卡在同一個地方。 | 沒有偵測長時間沒有輸出，也沒有辦法回答。 |
| GAP 5 | 升級途中連線中斷：錯誤只有「SSH 連線結束但沒有命令退出碼」，沒有提到升級可能仍在進行，升級步驟的輸出也沒有保存。沒有 PTY 時 apk 會把升級做完。無聲斷線約 210 秒才放棄。 | 與其他套件管理器相同。 |
| GAP 6 | apk 被訊號殺掉時，錯誤只有「遠端命令被訊號中止」，沒提升級被打斷，輸出也沒有保存。 | `ssh::execute` 收到 exit-signal 就直接回傳錯誤。 |

另外，升級被中斷後 MISF 的提示寫的是 `sudo dpkg --configure -a`，對 Alpine 主機不適用（`8aaa1b9` 的驗證紀錄已記載，沒有對應的測試）。

測試也確認了這些保護有效：索引打不開或簽章不受信任時，即使 apk 退出碼是 0，也存成失敗且不列出、不安裝任何東西；腳本印的 `ERROR:` 不會誤判；管理者改過的設定檔保留；沒有 `TERM` 讓 `whiptail` 直接失敗；免密碼 sudo 時 MISF 送出的密碼到不了套件腳本；不正常的輸出不會讓結果出錯或把密碼帶進輸出。

## 沒有涵蓋的範圍

- apk-tools 2（Alpine 3.22 以前）：輸出格式與索引錯誤的寫法不同，core 只有解析的單元測試。
- 真實的 Alpine 主機（OpenRC、需要重新開機的判斷）、`edge`、社群套件庫、`apk fix`、世界檔（`/etc/apk/world`）被改動的情況。
- 只用 doas 的主機（MISF 不支援，沒有測它失敗的樣子）。
- 套件庫是本機目錄，沒有測 HTTP 鏡像站變慢、逾時。
- 只有密碼登入，沒有測私鑰登入與批次作業。
