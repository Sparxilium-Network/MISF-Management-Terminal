# sshtest：APT 故障情境測試

> 最後修改：2026-11-16 ｜ 基於 commit：`0b85af9`

用真的 Debian／Ubuntu 容器測試 MISF 的 APT 檢查與升級：升級到一半失敗、斷線、不正常的輸出、套件腳本跳出畫面或等人回答。`core/tests/workflow.rs` 的模擬 SSH 伺服器只會回傳寫好的文字；這裡跑的是真的 sshd、sudo、apt、dpkg、debconf 與 needrestart。

每個測試都自己啟動一個全新的容器，結束時移除，所以可以任意順序、平行、重複執行，測試時不需要網路。

## 需要什麼

- 能執行 Linux 容器的 `docker` 指令（Windows 用 Docker Desktop，Linux 用 Docker Engine）。
- 第一次執行要建置映像：從 Docker Hub 下載基底映像，並從發行版的套件庫安裝 `openssh-server`、`sudo`、`whiptail`、`apt-utils`、`procps`、`needrestart`。之後由 Docker 的快取處理，約 1 秒。

## 執行

這些測試都標了 `#[ignore]`，平常的 `cargo test --workspace` 只會編譯、不會執行：

```powershell
cargo test -p misf-sshtest -- --ignored
```

| 環境變數 | 作用 |
| --- | --- |
| `MISF_SSHTEST_BASE` | 基底映像，預設 `debian:13`。例如 `ubuntu:26.04`。每個基底各有一個映像 `misf-sshtest:<基底>`。 |
| `MISF_SSHTEST_SLOW=1` | 另外執行約 4 分鐘的「網路無聲中斷」測試；沒設定時該測試直接通過並印出略過的說明。 |

不含慢速測試約 45 秒（17 項平行執行）。只跑一項：`cargo test -p misf-sshtest -- --ignored a_cut_connection`。

測試程序被強制結束時（例如按 Ctrl+C），容器來不及移除。列出與清除：

```powershell
docker ps -aq --filter label=misf-sshtest=1
docker rm -f (docker ps -aq --filter label=misf-sshtest=1)
```

## 接到 CI

目前沒有接進 GitHub Actions。要接的時候，在 Linux runner（`ubuntu-latest` 已內建 Docker）裝好 Rust 與 `build-prerelease.yml` 的「Install Linux build dependencies」之後加一步：

```yaml
- name: APT fault tests
  run: cargo test --locked -p misf-sshtest -- --ignored
```

Windows runner 不能執行 Linux 容器，所以只能放在 Linux 工作。這些測試會從 Docker Hub 與發行版套件庫下載，外部服務故障時會失敗；建議先用獨立、手動觸發的 workflow，不要擋住發布。

## 怎麼運作

```text
測試 ──► misf-core（dispatch_with）──SSH──► Relay（127.0.0.1）──► 容器的 sshd ──► sudo／apt-get／dpkg
  └────────── docker exec：佈置情境、觀察 dpkg 狀態 ──────────────────────────────►
```

- `docker/Dockerfile`：sshd（只有 Ed25519 主機金鑰）、三個帳號、本機套件庫。APT 來源只剩 `file:/opt/misf/repo`。
- `docker/build-packages.sh`：建出測試用套件。每個套件的 1.0 版已安裝在映像裡，2.0 版放在 `/opt/misf/pool/v2`，各自以一種方式搗亂。
- `docker/misf-scenario`：把指定套件的 2.0 版發布到本機套件庫（`Lab::start` 的第三個參數）。
- `src/lib.rs`：`Container`（啟動、`docker exec`、讀 dpkg 狀態）、`Relay`（MISF 與容器之間的 TCP 中繼，`cut` 立刻斷線、`freeze` 不關閉但不再傳資料、`restore` 恢復）、`Lab`（像 app 一樣把主機存進主機庫、信任指紋、測試連線，再用 `dispatch_with` 執行 `check`／`upgrade`／`dist_upgrade`，並收集即時終端機輸出）。
- `Lab::run` 最多等 5 分鐘（`PATIENCE`），超過就讓測試失敗，不會等到 MISF 自己的 30 分鐘上限。`Lab::run_for` 在指定秒數後放棄並斷開連線，效果等同 app 的「中斷」。

帳號（密碼寫在 Dockerfile，只是測試值；SSH 埠只發布在 127.0.0.1）：

| `Account` | 帳號 | 說明 |
| --- | --- | --- |
| `Root` | `root` | 直接執行 APT |
| `Admin` | `misfadmin` | 經 sudo，要輸入密碼 |
| `Deploy` | `misfdeploy` | 經 sudo，`NOPASSWD` |

測試套件：

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

新增情境：在 `build-packages.sh` 加一個套件（`start`／`script`／`finish`），把名稱加進安裝 1.0 版的迴圈，再在 `tests/apt.rs` 用 `Lab::start(帳號, 是否使用 PTY, &["套件"])` 寫測試。

## 測試項目

| 測試 | 情境 |
| --- | --- |
| `a_clean_upgrade_through_sudo_…` | 基準：檢查、升級、真實 needrestart 輸出、reboot-required |
| `a_failing_package_script_…` | 升級到一半失敗，一個套件完成、一個停在 half-configured |
| `questions_apt_would_ask_…` | debconf 問題與 conffile 衝突都不會跳出來 |
| `kept_back_packages_…` | kept back、dist-upgrade 模擬計畫、dist-upgrade |
| `a_broken_repository_…` | 套件庫索引抓不到 |
| `a_package_file_that_cannot_be_fetched_…` | 套件檔不見、套件檔被截斷 |
| `garbage_from_a_package_script_…` | 不正常的輸出 |
| `a_script_line_that_looks_like_an_apt_error_…` | 腳本印出 `E:` 行 |
| `root_ignores_…marker`、`the_marker_after_sudo_…`、`with_passwordless_sudo_…` | 輸出裡出現 sudo 提示標記（root、要密碼的 sudo、免密碼 sudo） |
| `a_dialog_cannot_open_on_the_dumb_terminal_…` | 對話框在 `TERM=dumb` 下開不起來 |
| `a_drawn_dialog_keeps_the_upgrade_waiting_…` | TUI 畫面真的出現 |
| `a_script_waiting_for_an_answer_…` | 腳本等待鍵盤回答 |
| `a_cut_connection_…` | 升級途中斷線 |
| `dpkg_killed_half_way_…` | 升級途中 dpkg 被殺掉 |
| `an_upgrade_while_another_apt_holds_the_lock_…` | 另一個 apt 佔著鎖 |
| `a_silently_dead_link_…`（慢） | 網路無聲中斷，沒有人關閉連線 |

## 測試發現的缺口

測試的斷言寫的是 MISF **目前**的行為。下列項目不是期望的行為，在 `tests/apt.rs` 以 `GAP n` 標示；日後修正 core 時要一併修改對應的斷言。2026-11-16 在 `debian:13` 與 `ubuntu:26.04` 上結果相同。

| 編號 | 現象 | 原因 |
| --- | --- | --- |
| GAP 1 | 升級半途失敗（postinst 失敗、dpkg 被殺、被對話框卡住）之後，再檢查或再升級都顯示「成功、沒有待更新套件」，主機看起來是最新的，但 `dpkg --audit` 仍回報 half-configured。 | 已解開但未設定完成的套件不再列在 `apt list --upgradable`；清單是空的，MISF 就不執行 `apt-get upgrade`，看不到 APT 的 `dpkg was interrupted` 錯誤，也沒有另外檢查 dpkg 狀態。 |
| GAP 2 | 套件腳本印出沒有結尾的跳脫序列（`ESC ]`…）後，該命令之後的輸出在保存的紀錄裡全部消失。即時終端機只少那一行。 | `transcript::readable_output` 把 OSC 字串一路吃到結尾符號，整段命令輸出一次處理；即時輸出是逐行處理。 |
| GAP 3 | 套件腳本印出以 `E:` 開頭的一行，APT 退出碼 0、套件已裝好，結果卻存成「套件升級失敗（退出碼 0）」。 | `validate_upgrade` 把任何 `E:`／`Error:` 開頭的行都當成 APT 錯誤。 |
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
- 只測 APT；DNF、YUM、pkg 沒有對應的容器。
- 只有密碼登入，沒有測私鑰登入與批次作業。
