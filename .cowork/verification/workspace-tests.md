# 全工作區檢查結果

## 2026-11-18（chrono 停止維護公告暫時忽略）

- 擁有者決定暫時忽略 RUSTSEC-2026-0543（chrono 不再維護、無修補版本），在 `deny.toml` 加入附原因的例外，並在 `docs/dependency-updates.md` 說明等待 chrono 棄用與替代方案後再處理。
- `cargo deny --locked check`：`advisories ok, bans ok, licenses ok, sources ok`。
- 沒有程式碼變動，未重跑測試與打包。

## 2026-11-18（rusqlite 0.42.0，Dependabot PR #7，於 `main`）

- 使用者要求套用 PR #7 並合併到主分支：`main`（`47a9b18`）先快轉到 `feat/WYSIWYG-Terminal`（`21a90e7`），再於 `main` 手動套用與 PR 相同的變更（`core/Cargo.toml`、`Cargo.lock` 中 rusqlite 0.41.1 → 0.42.0、libsqlite3-sys 0.39.1 → 0.40.0），沒有其他鎖檔變動；`cargo metadata --locked` 通過；重跑 `./licenses.ps1`。
- 檢查：fmt、Clippy（`--all-features --locked -D warnings`）通過；`cargo test --locked --workspace` 通過（22、37、72、33）；`cargo deny check licenses` 通過。
- `cargo deny check advisories` 失敗：新公告 RUSTSEC-2026-0543（chrono 不再維護，沒有可升級版本），與這次升級無關（還原變更後同樣失敗）。chrono 由 Slint 1.18.1、`misf`、`misf-core` 使用。待使用者決定是否在 `deny.toml` 加入附理由的忽略。
- 未驗證：rusqlite 0.42 對既有加密主機庫的實際讀寫只靠測試（含 sqlite／vault 測試），沒有用真實主機庫開啟。

## 2026-11-18（主機頁卡片合成一張、可在偏好設定選項目，分支 `feat/WYSIWYG-Terminal`，基於 `eb2215f`）

- 使用者要求：刪除「主機總覽」小字；右上角卡片合成一張；偏好設定可自訂顯示項目。
- 實作：`Stat` 改為同一張 `Card` 內的區塊，以分隔線隔開；`HostState.show-*` 控制每一項與連線資訊行，四項都隱藏時不畫卡片。偏好 `HostStats`（`ui-settings.json` 的 `host_page.stats`，沒有這個鍵時全部顯示），設定在「外觀與主機清單」頁新增「主機頁」區段的勾選框。移除語系鍵 `app.host_overview`，新增 `settings.section_host_page`、`host_stats.title`、`host_stats.connection`。
- 檢查：fmt、Clippy（`-D warnings`）通過；`cargo test --workspace` 通過（22、37、72、33）。擷取主機頁與偏好設定頁確認版面。
- 未驗證：取消勾選部分項目後的版面（只依程式確認）；偏好的讀寫沒有新增測試（測試由他人負責）。

## 2026-11-18（主機頁卡片移到右上角、新增延後套件卡片，分支 `feat/WYSIWYG-Terminal`，基於 `6913d34`）

- 使用者要求：待更新套件、檢查狀態、上次檢查與主機連線資訊移到右上角，新增「延後套件」，移除分階段推送的說明行。使用者選擇卡片與連線資訊（帳號@位址・系統・套件管理器・登入方式）都放右側，左側留名稱、按鈕與指紋。
- 實作：`HostPage` 標題區改成左右兩欄；`Stat` 縮小；新增 `HostState.phased-count`（沒有成功結果時為「—」）。`HostState.phased` 保留原本的句子，改當「延後套件」卡片的滑鼠提示，所以既有 UI 測試讀 `get_phased()` 的斷言不受影響。新增語系鍵 `apt.phased_count`。擷取用的選單座標跟著按鈕新位置調整。
- 檢查：fmt、Clippy（`-D warnings`）通過；`cargo test --workspace` 通過（22、37、72、33）。以 1280×820 擷取主機頁確認版面。
- 未驗證：英文介面與接近最小視窗寬度（1024）時的版面；卡片滑鼠提示只依程式確認。

## 2026-11-18（套件篩選改為放大鏡、移除表格說明，分支 `feat/WYSIWYG-Terminal`，基於 `3077664`）

- 使用者要求：「篩選套件名稱」縮成放大鏡，按下才顯示篩選框；移除「以下套件…」說明。
- 實作：`ResultPanel` 的放大鏡按鈕（`ResultState.search-open`），篩選框在開啟或有篩選文字時顯示，開啟時取得焦點；再按一次清除篩選並收起。移除 `apt.available_description`、`apt.upgraded_description`，訊息為空時不顯示該行；錯誤、「已是最新」、「升級後仍待更新」等訊息照舊。新增擷取旗標 `MISF_OPEN_SEARCH=1`。
- 檢查：fmt、Clippy（`-D warnings`）通過；`cargo test --workspace` 通過（22、37、72、33）。擷取主機頁的收起與展開兩種狀態確認版面。
- 未驗證：放大鏡的點擊與焦點只依擷取畫面與程式確認，沒有新增 UI 測試。

## 2026-11-18（主機頁「其他操作」選單，分支 `feat/WYSIWYG-Terminal`，基於 `b4afaf8`）

- 使用者要求：主機指紋、重新載入、刪除主機、編輯主機收進「其他操作」選單，放在「進階操作」右側。
- 實作：`host_page.slint` 抽出共用 `MenuFrame`，新增 `MoreMenu`；`HostState.more-open`／`more-toggled`、`App.more_open`（鎖定與無主機時關閉）。標題列的編輯與刪除按鈕、動作列的主機指紋與重新載入按鈕移除。新增語系鍵 `more.button`、`host.delete`；擷取旗標 `MISF_OPEN_MENU=more`。
- 檢查：fmt、Clippy（`-D warnings`）通過；`cargo test --workspace` 通過（22、37、72、33）。以 `MISF_OPEN_MENU=more` 擷取確認版面。
- 未驗證：選單項目只依擷取畫面與程式確認，沒有新增 UI 測試（測試由他人負責）。

## 2026-11-18（以含分階段推送的模擬判斷連帶保留，分支 `feat/WYSIWYG-Terminal`，基於 `7678ab9`）

- 使用者以 `temp/misf-log-20261118-004128.txt` 回報數字仍不一致（agent-claude-vm 69／53）。log 顯示 `apt-get -s dist-upgrade` 仍把 16 個 Plasma 套件列為 kept back，沒有列為延後，所以 `3f6ce34`、`7678ab9` 的判斷從未成立。
- 使用者在該主機另外執行 `apt-get -s -o APT::Get::Always-Include-Phased-Updates=true upgrade`：16 個中有 14 個變成會升級，剩 `plasma-session-wayland`、`plasma-workspace` 仍保留。
- 改為：`SIMULATE` 仍不會升級某些被保留的套件時，再跑唯讀的 `apt::SIMULATE_PHASED`；它會升級而 `SIMULATE` 不會的，算成分階段延後。檢查與升級共用 `settle_kept`。讀回時以 `transcript::section` 分開兩段模擬（`apt::phased_in_transcript`），`plan` 只取 `SIMULATE` 那段。
- 用 log 裡的真實檢查輸出加上使用者貼的模擬輸出驗證解析：分階段延後 53 → 67，仍被保留的只剩上述 2 個。
- 檢查：fmt、Clippy（`-D warnings`）通過；`cargo test --locked --workspace` 通過（22、37、72、33）；`sshtest` APT 18 項通過。`.\package.ps1` 雜湊相同。
- 未驗證：MISF 實際以 dist-upgrade（而非使用者跑的 upgrade）加上這個選項的結果，剩下 2 個套件可能也會被判成延後；`SIMULATE_PHASED` 只在需要時執行，沒有列在確認視窗的命令清單（列出會讓既有 workflow 測試的命令比對失敗）。

## 2026-11-17（套件明細改為工具視窗；檢查時也確認分階段連帶保留，分支 `feat/WYSIWYG-Terminal`，基於 `3f6ce34`）

- 使用者要求：移除批次表格下方的上一台／下一台、「X 的套件」標題與明細區塊，改成左側工具列的「套件明細」工具視窗，並可從右鍵開啟。使用者選擇移除「批次作業套件明細」設定，右鍵同時保留「在套件明細欄開啟」與「在視窗中顯示套件明細」（後者原本只有設定為視窗模式才啟用，所以是灰的）。
- 實作：`Tool::Packages`（索引 3）、`PackagesPanel` 顯示目前主機的 `ResultPanel`；`BatchState.open-dock`。移除 `BatchDetail` 偏好（舊設定檔的 `batch_detail` 鍵讀取時忽略）。因此 `preferences.rs` 與 `ui/tests.rs` 的測試刪掉設定 `batch_detail` 的幾行，其他斷言未動。
- 使用者回報檢查後「待更新」與「分階段延後」不一致（例如 69／53）：檢查只用 `apt-get -s upgrade` 找分階段延後，被連帶保留的套件不算。現在檢查的模擬有被保留的套件時，再跑唯讀的 `apt-get -s dist-upgrade`，其中延後的一併算成分階段延後；檢查的命令清單（確認與提示）也列出這一步。
- 檢查：fmt、Clippy（`-D warnings`）通過；`cargo test --locked --workspace` 通過（22、37、72、33）；`sshtest` APT 18 項通過。以新增的 `MISF_OPEN_PACKAGES=1` 擷取批次頁與套件明細工具視窗確認版面。`.\package.ps1` 雜湊相同。
- 未驗證：真實 Ubuntu 主機上 `apt-get -s dist-upgrade` 是否把那些連帶保留的套件印成延後（沒有輸出可對照）；介面上右鍵選單只依程式與既有 UI 測試確認。

## 2026-11-17（分階段推送連帶保留的套件不算 kept back，分支 `feat/WYSIWYG-Terminal`，基於 `b364e83`）

- 使用者貼上 Ubuntu 26.04 主機的升級輸出：73 個 Plasma／GTK 套件全部 `deferred due to phasing`。以 core 解析確認判讀正確（phased 73、kept back 0）。先前報告裡該主機的被保留套件都在這 73 個之中，推斷是被分階段推送連帶保留，dist-upgrade 也裝不了。使用者同意改 core。
- `operation.rs`：dist-upgrade 模擬另外讀出分階段延後的套件；被保留套件若在其中，改記到 `phased`，剩下的才是 kept back，沒有剩下時不保留 `plan`。`sqlite.rs` 讀回時同樣以 `phased` 過濾 kept back。
- 檢查：Clippy（`-D warnings`）、fmt 通過；`cargo test --locked --workspace` 通過（22、37、72、33）；`sshtest` APT 18 項通過（`misf-kept` 因新相依被保留的情境不受影響）。`.\package.ps1` 雜湊相同。
- 未驗證：真實主機上 `apt-get -s dist-upgrade` 是否真的對那些連帶保留的套件印出 `deferred due to phasing`（沒有當時的輸出）；若它仍列為 kept back，這次改動不會生效，報告仍以「模擬沒有變動、預設不勾選」顯示。沒有新增測試（測試由他人負責）。

## 2026-11-17（dist-upgrade 報告改版，分支 `feat/WYSIWYG-Terminal`，基於 `fae9a89`）

- 使用者回報報告畫面問題並決定新設計：不能點外面關閉，只有「執行 dist-upgrade」與「取消」；可勾選要做的主機；不顯示被保留的套件，改由「詳細情況」開啟獨立視窗；報告本身就是確認（不再跳第二個確認視窗）。
- 原畫面第二台主機被截斷且沒有捲軸：`InfoLines` 的換行文字在 `for`／`if` 內，Slint #13655 以單行量高度，捲動區內容高度被低估。已在 `InfoLines` 的換行元素加 `min-height: self.preferred-height`。
- 「跳出兩次」：dist-upgrade 結果若仍有 kept back（例如分階段推送，dist-upgrade 也不裝），報告會再跳一次。現在只有 `upgrade` 的結果會開報告，dist-upgrade 之後只在狀態列列出主機。使用者截圖中「無法取得模擬結果」的那台實際原因沒有確認（主機庫加密，讀不到輸出）；新畫面把這種情況說明為「模擬沒有變動」，預設不勾選，原因可在詳細情況的模擬原始輸出看到。
- 檢查：fmt、Clippy（`-D warnings`）通過；`cargo test --locked --workspace` 通過（22、37、72、33）。以新增的 `MISF_OPEN_KEPT_BACK=1`／`details` 擷取報告與詳細情況視窗，確認版面。`.\package.ps1` 成功，`dist\MISF\misf.exe` 與 `target\release\misf.exe` 雜湊相同。
- 未驗證：在真實主機上走完「升級 → 報告 → 勾選 → dist-upgrade」；Esc／點外面不關閉只依程式判斷，沒有實際操作；沒有新增 UI 測試（測試由他人負責）。

## 2026-11-17（新增 docs/test 測試清單，分支 `feat/WYSIWYG-Terminal`，基於 `546e058`）

- 使用者要求 `docs/test/` 下每個套件管理器一份清單（`apt-test.md`、`dnf-test.md`、`pacman-test.md`、`apk-test.md`），列出測試項目與所在檔案，另有總覽 `README.md` 說明怎麼只跑一部分。
- 清單涵蓋三層：單元測試、`core/tests/workflow.rs`、`sshtest/<管理器>/`（含帳號與 GAP 編號）。不屬於某個套件管理器的 16 項 workflow 測試與涵蓋四個管理器的 5 項單元測試列在總覽。
- 文件裡的每個選取指令都以 `--list` 實跑核對項數：單元 APT 12、DNF 8、pacman 8、apk 7；workflow APT 10、DNF 6、pacman 2、apk 2；sshtest 18、22、23、22。APT 的單元與 workflow 指令另實際執行通過（PowerShell 陣列傳名稱給 `--exact` 可用）。容器測試沒有執行。
- `docs/test/` 不在 `package.ps1` 複製的範圍（只複製 `docs/*.md`），不會進發行包。

## 2026-11-17（寫清楚測試分工，分支 `feat/WYSIWYG-Terminal`，基於 `eebbd7a`）

- 使用者決定保留 `core/tests/workflow.rs`，不與 `sshtest/` 合併：程式本體自己的行為由各 crate 的測試負責，SSH 連到真實主機時實際會遇到的狀況由 `sshtest/` 負責。
- `docs/PROJECT-DEV.md` 的「測試」新增「分工」表與放置原則；`docs/PROJECT-DEV.md`、`docs/architecture.md` 的整合測試描述補上 pacman 與 apk（上一次提交漏改）。
- 只改文件，沒有執行建置或測試。

## 2026-11-17（sshtest 新增 pacman 與 apk，分支 `feat/WYSIWYG-Terminal`，基於 `8aaa1b9`）

- 使用者要求新增 pacman 與 apk 的容器測試，並說明 core 的支援已由另一個工作階段完成（`8aaa1b9`）。該工作階段留下未追蹤、未接線的草稿：`sshtest/apk/`（映像與 16 項測試）與 `sshtest/pacman/`（只有映像）。本次接手：在 `src/lib.rs` 加上 `PACMAN`、`APK`，`Cargo.toml` 加兩個測試目標。
- apk：草稿 16 項第一次執行就全過。兩個映像都加了 `misf-dialog`、`misf-tui`、`misf-tty` 三個測試套件，`misf-noise` 多兩行（非 UTF-8、沒有結尾的跳脫序列）；草稿裡因此改變的斷言已更新。新增 `prompts.rs`、有 PTY 時斷線、套件檔讀不到、無聲斷線，鎖與被殺兩項改成明確的斷言。共 22 項。
- pacman：測試全部新寫（先以探索測試觀察行為）。共 23 項。
- 結果：pacman 連續 4 次、apk 連續 4 次全過（不含慢速項分別約 40 秒、70 秒）。`MISF_SSHTEST_SLOW=1 cargo test --locked --workspace -- --include-ignored` 全過 1 次：luksbox-fido2 22、misf 37、misf-core 72、workflow 33、apk 22、apt 18、dnf 22、pacman 23（四個容器目標各約 215 秒，依序執行），沒有殘留容器。`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo fmt --all -- --check` 通過。APT 與 DNF 的容器測試在 `8aaa1b9` 改過錯誤行判斷之後仍然全過。
- pacman 的發現（`sshtest/pacman/README.md`，9 個缺口）：最重要的是過期鎖檔——pacman 被殺，或沒有 PTY 時連線中斷，`/var/lib/pacman/db.lck` 會留下，之後有更新時升級一律失敗直到手動刪除，而當下檢查顯示正常。其他：套件庫抓不到時輸出看不出原因且錯誤訊息誤附「需要安裝 pacman-contrib 與 fakeroot」；腳本印 `error:` 行會把成功的升級存成失敗；其餘與 APT／DNF 相同（提示標記誤判、跳脫序列吃掉輸出、終端機上等人、斷線與訊號的錯誤訊息）。
- apk 的發現（`sshtest/apk/README.md`，6 個缺口）：apk 最後才寫資料庫，有 PTY 時斷線或被殺，這一輪全部不算，下次整批重做、腳本重跑，沒有紀錄；卡在終端機對話框的套件中斷後仍待更新，下次又卡住。apk 比較好的地方：腳本輸出有前綴所以 `ERROR:` 不會誤判、鎖隨程序釋放。另一個 apk 佔著鎖時連檢查都會失敗（`apk update` 退出碼 99）。
- 兩份 README 各有「特殊行為」一節，整理給日後修改 core 的人。
- 未驗證：真實 Arch／Alpine 主機；apk-tools 2；`archlinux:latest` 是滾動發行，日後重建映像時 pacman 的版本與輸出可能改變；pacman 的套件簽章、`IgnorePkg`、套件衝突時的提問。`ubuntu:26.04`、`almalinux:9` 這次沒有重跑。

## 2026-11-17（新增 pacman、apk，移除 YUM 與 BSD，分支 `feat/WYSIWYG-Terminal`，基於 `fe385c8`）

- 使用者決定：舊主機保留並改回自動偵測（`Host::forget_unsupported`，讀取時處理、不做資料庫升級）；編輯主機的 OS 選單保留但只剩 Linux；pacman 檢查用 `checkupdates`（需 pacman-contrib 與 fakeroot，不經 sudo、不動系統資料庫）；apk 只支援 root 或 sudo。
- core：新增 `pacman.rs`、`apk.rs`，刪除 `yum.rs`、`pkg.rs`；偵測只接受 Linux；升級的錯誤行改依管理器判斷（APT `E:`、DNF `Error:`、pacman `error:`、apk `ERROR:`）；apk 索引警告視為更新失敗；pacman 更新失敗時提示需要的套件。
- UI：管理器選單改為 APT／DNF／pacman／apk；pacman 的檢查說明另寫（不使用 sudo）；語系移除 BSD／YUM／pkg。
- 測試：既有測試改掉 YUM／BSD 的例子；新增 pacman、apk 的單元測試與 loopback 整合測試（workflow 33 項），以及舊紀錄改回自動偵測的測試。`cargo test --workspace`（luksbox-fido2 22、misf 37、misf-core 72、workflow 33）、兩種 clippy、`cargo fmt --check` 通過。
- 使用者說明測試另有人寫，sshtest 的 pacman／apk 容器測試未納入（草稿留在工作目錄，未追蹤）。實作前曾以 Arch 與 Alpine 3.24（apk-tools 3.0.9）容器確認命令與輸出格式。
- 未驗證：真實 Arch／Alpine 主機；apk-tools 2（Alpine 3.22 以前）只依文件與解析測試；新選單與說明的實際畫面。升級中斷後的提示仍寫 `dpkg --configure -a`，對 pacman／apk 不適用，沒有改。

## 2026-11-17（sshtest 新增 DNF，分支 `feat/WYSIWYG-Terminal`，基於 `ffff655`）

- 使用者要求新增 DNF 版本。新增 `sshtest/dnf/`：`docker/`（預設 `rockylinux/rockylinux:9`，以 rpmbuild 建出 16 個測試套件、`createrepo_c` 本機套件庫）、`support.rs`（`Rpm`）、五個主題檔共 22 項測試、`README.md`。`src/lib.rs` 新增 `DNF`，並把通用斷言（`names`、`assert_plain`、`assert_failed`）從 `apt/support.rs` 移到共用框架。基底映像沒有先問使用者，選 Rocky Linux 9 是因為既有模擬測試與文件都以它為例。core 與 app 沒有改，沒有新增依賴，`Cargo.lock` 沒有變。
- 結果：`cargo test -p misf-sshtest --test dnf -- --ignored` 在 Rocky Linux 9.8（dnf 4.14）連續 3 次全過（約 45 秒），`MISF_SSHTEST_SLOW=1` 的慢速項單獨通過 1 次（214 秒），`almalinux:9` 1 次全過。`cargo test --locked --workspace -- --include-ignored` 全過（22、37、70、29 項，APT 18 項，DNF 22 項），沒有殘留容器；clippy（`--all-features -D warnings`）、`cargo fmt --check` 通過。
- `fedora:latest`（dnf5 5.4）不能用：新版 rpm 拒絕未簽章的測試套件（已在映像改用 `_pkgverify_level none`），之後 22 項仍全部失敗，原因是 MISF 讀不懂 dnf5 的清單（`check-update` 先印一行 `Upgrades`），回報「無法解析 DNF／YUM 套件清單」；映像也還留著 Fedora 自己的套件庫快取。沒有再往下處理。
- 測試發現 11 個缺口（`sshtest/dnf/README.md`，編號只在 DNF 內有效）：`%post` 失敗仍存成成功；`skip_if_unavailable` 的套件庫抓不到時檢查不會發現；`Error:` 開頭的行讓成功的升級存成失敗；dnf 被訊號殺掉時錯誤沒提升級、輸出未保存；之後留下新舊兩版重複安裝而沒有提示；未結束的跳脫序列吃掉之後保存的輸出；sudo 收下密碼後的提示標記被當成密碼錯誤；腳本在 `/dev/tty` 上等人時等到 30 分鐘上限；斷線時錯誤沒提升級可能進行中；經 sudo 加 PTY 斷線時腳本被中途結束但套件算已安裝；dnf5 清單無法解析。都還沒有處理。
- 與 APT 不同且較好的地方也寫進 README：rpm 不給腳本標準輸入（`read` 不會卡住、密碼到不了腳本）、`%pre` 失敗的套件仍在待更新清單、dnf 會等鎖、中斷後不會留下卡住的程序。
- 未驗證：Fedora／dnf5、RHEL 8、HTTP 套件庫、GPG 簽章錯誤、`best=False`；GitHub Actions 上的執行。
- 殘留：試 Fedora 留下的映像 `misf-sshtest-dnf:fedora-latest` 可以刪除。

## 2026-11-16（文件補上「執行所有測試」，分支 `feat/WYSIWYG-Terminal`，基於 `752ab8d`）

- 使用者要求把如何執行所有測試寫進文件。`docs/PROJECT-DEV.md` 的「測試」新增「執行所有測試」小節與指令對照表，`sshtest/README.md` 補上同一條指令。只改文件。
- 實測：`cargo test --locked --workspace -- --include-ignored` 全過（22、37、70、29 項，加上容器測試 18 項），執行後沒有殘留容器。加上 `MISF_SSHTEST_SLOW=1` 的組合沒有在這次重跑（慢速項先前單獨通過）。

## 2026-11-16（sshtest 依套件管理器分資料夾，分支 `feat/WYSIWYG-Terminal`，基於 `416718d`）

- 使用者要求：之後會增加其他套件管理器，測試要像 `apt/…`、`yum/…` 這樣分開，不能擠在一起。
- `sshtest/docker/` 與 `sshtest/tests/apt.rs` 搬到 `sshtest/apt/`（`docker/`、`main.rs`、`support.rs`，測試依主題拆成 `upgrade.rs`、`failures.rs`、`output.rs`、`prompts.rs`、`connection.rs`）；`Cargo.toml` 以 `[[test]] name = "apt"` 指向 `apt/main.rs`。`src/lib.rs` 只留共用框架：新增 `System`／`APT`，映像依管理器建置（`misf-sshtest-<管理器>:<基底>`），dpkg 專用的輔助移到 `apt/support.rs`。環境變數 `MISF_SSHTEST_BASE` 改為 `MISF_SSHTEST_APT_BASE`。說明拆成 `sshtest/README.md`（共用、如何新增套件管理器）與 `sshtest/apt/README.md`（APT 的套件、測試項目、缺口）。測試內容與斷言沒有改。
- 結果：`cargo test -p misf-sshtest --test apt -- --ignored` 在 `debian:13` 全過（18 項，約 45 秒，慢速項略過）；`cargo test --workspace`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo fmt --all -- --check` 通過。
- 未驗證：搬動後沒有重跑 `ubuntu:26.04` 與 `MISF_SSHTEST_SLOW=1`（測試內容未變）。`package.ps1` 因使用者的 `dist\MISF\misf.exe` 正在執行而無法覆寫；app 的程式碼沒有改。
- 殘留：舊名稱的映像 `misf-sshtest:debian-13`、`misf-sshtest:ubuntu-26-04` 可以刪除。

## 2026-11-16（sshtest：真實容器的 APT 故障測試，分支 `feat/WYSIWYG-Terminal`，基於 `0b85af9`）

- 使用者要求在 `sshtest` 資料夾做可重複執行的 APT 自動化測試（升級到一半失敗、斷線、不正常的輸出、出現 TUI 畫面等），供日後 CI 使用。使用者選擇：Docker 真實 APT、workspace 成員 crate（測試標 `#[ignore]`）、發現問題只記錄並回報不改 core、這次不接 GitHub Actions 只寫文件。
- 新增 crate `misf-sshtest`（`sshtest/`）：`docker/` 是映像（sshd、三個帳號、本機套件庫與 13 個會搗亂的測試套件），`src/lib.rs` 是框架（容器、TCP 中繼、`Lab`），`tests/apt.rs` 有 18 項測試。沒有新增第三方依賴；`Cargo.lock` 只多了這個 crate 的條目，`./licenses.ps1` 重新產生戳記。core 與 app 的程式碼沒有改。
- 結果：`cargo test -p misf-sshtest -- --ignored` 在 `debian:13` 連續 4 次全過（17 項約 45 秒；加上 `MISF_SSHTEST_SLOW=1` 的 18 項約 217 秒，跑過 1 次），`MISF_SSHTEST_BASE=ubuntu:26.04` 1 次全過（不含慢速項）。執行後沒有殘留容器。`cargo test --workspace`（22、37、70、29 項，sshtest 18 項 ignored）、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo fmt --all -- --check` 通過。
- 測試發現 8 個缺口（斷言記錄的是目前行為，標為 `GAP n`，詳見 `sshtest/README.md`）：升級半途失敗後再檢查顯示成功且沒有待更新套件；未結束的跳脫序列吃掉之後保存的輸出；腳本印出 `E:` 行讓成功的升級存成失敗；sudo 收下密碼後輸出出現提示標記被當成密碼錯誤；免密碼 sudo 時密碼送進套件腳本（使用指南已記載）；腳本等待輸入或畫出對話框時等到 30 分鐘上限；中斷後遠端 apt-get 仍卡著並持有鎖；斷線時錯誤沒提升級可能進行中、該步驟輸出未保存，遠端照常升級完，無聲斷線約 210 秒才放棄。都還沒有處理，等使用者逐項決定。
- 也確認有效：`DEBIAN_FRONTEND=noninteractive` 經 `sudo -- env` 仍生效、`--force-confold`、`TERM=dumb` 讓 whiptail 直接失敗、各種失敗都存成失敗、不正常輸出不會洩漏密碼、真實 `needrestart -r l -b`（3.11）與 reboot-required 解析正確、kept back 與 dist-upgrade 模擬計畫。
- 過程中修正的測試套件問題（不是 MISF 的問題）：Ubuntu 26.04 的 `timeout` 是 uutils 版，背景讀取終端機的程序被停住後不會被喚醒，測試卡住；改成不用 `timeout`。
- 未驗證：GitHub Actions 上的實際執行（沒有接）；sudo-rs；HTTP 套件庫的逾時與簽章錯誤；phased updates；有 systemd 的主機上的 needrestart 服務清單；私鑰登入與批次作業在真實容器上的行為。
- 殘留：探索時被強制停止的一次 Ubuntu 執行留下一個容器（`docker ps --filter label=misf-sshtest=1`），移除指令被安全檢查擋下，留給使用者移除。

## 2026-11-16（MYTHOS-r4 的處理，分支 `feat/WYSIWYG-Terminal`，基於 `8b1a81f`）

- 使用者決定：第 1、4、7 項為預期風險；第 5 項暫不處理；第 8 項之後再說；低風險項目暫不處理。
- 第 2 項：新增中斷（`core/src/cancel.rs`）。檢查、升級、dist-upgrade、批次、測試連線可中斷，重新開機不行；升級類先確認並說明 dpkg 風險，被中斷的升級存為結果未知；忙碌時按鎖定會詢問「操作結束後鎖定」或「中斷並鎖定」（使用者選擇手動鎖定時詢問，自動鎖定不中斷）。
- 第 3 項：主機庫鎖定時停用 LOG 匯出（按鈕停用，`export_log` 再檢查並記 `blocked`）。
- 第 6 項：root 不傳 sudo 密碼；`ssh::execute` 只在命令本身帶提示字串時回應。免密碼 sudo 寫進使用指南（使用者選擇只改 root）。
- 測試：core 新增 root 輸出提示字串不送密碼、單台升級與檢查中斷、批次中斷保留已完成主機（workflow 29 項）；UI 新增中斷檢查、升級確認與鎖定兩種選擇、批次中斷計數、鎖定時匯出被擋（misf 37 項）。`cargo test --workspace`、兩種 clippy、`cargo fmt --check` 通過。
- 未驗證：狀態列「中斷」按鈕與 StopDialog 的實際畫面；真實主機上中斷升級的結果。

## 2026-11-16（shell 檢查第四輪的回覆，分支 `feat/WYSIWYG-Terminal`，基於 `02068aa`）

- 使用者決定：自動鎖定預設只對全新設定生效不需處理（開發階段）；shell 持續輸出時不自動鎖定維持現狀，寫進使用指南。
- 修正：模擬器在繪製畫面時失敗也會送出關閉要求；分頁按 × 時，只要 shell 仍連線就先送關閉要求（結束事件到達後記稽核、標示分頁，再按一次才移除）；模擬器失敗後結束時保留「模擬器無法處理」的說明。
- 未處理：鎖定時終端機內容直接丟棄、不先 zeroize。`vt100` 的畫面與歷史在套件內部，無法逐格抹除；與過往操作的處理方式一致。
- 測試：UI 測試新增「連線中的失敗分頁按 × 先送關閉、結束後才移除並記稽核」。`cargo test --workspace`、clippy、`cargo fmt` 通過。

## 2026-11-16（即時輸出單行上限與 sudo 提示計數，分支 `feat/WYSIWYG-Terminal`，基於 `d3f9ecc`）

- 依使用者提供的修法調整（MYTHOS-r2 遺留項目）：
  - `console::Lines`：`\r` 也是行尾（資料結尾的 `\r` 與下一段開頭的 `\n` 合併），只掃描新資料；單行上限 8 KiB，超過時保留「上限＋最長密碼長度」原始位元組 → 清理 → 遮蔽 → 丟掉結尾最長密碼長度個字元 → 超過上限再切 → 加上「（此行過長，已截斷）」，之後丟棄到下一個行尾。
  - `ssh::execute`：sudo 提示改由 `count_prompts` 增量計數（新資料加上前一段結尾提示長度減一個位元組），「一次送密碼、第二次中止」不變。
- 互動 shell 不受影響。存檔的完整輸出不變。
- 測試：console 新增 5 個（長行截斷且下一行完整、密碼跨截斷點、幾乎全是控制序列時密碼跨截斷點、中文字跨截斷點不留替代字元、`\r` 進度列即時送出且 `\r\n` 跨兩段不多空行）；ssh 新增 1 個（提示拆成多段、逐位元組、同段兩次、與 `str::matches` 結果一致）；既有的「sudo 提示拆在兩個封包」與「重複提示中止」整合測試通過。`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt` 通過。

## 2026-11-16（處理 MYTHOS-r2 的問題，分支 `feat/WYSIWYG-Terminal`，基於 `0ac559b`）

- 依 `.daybrake/dangerzone/MYTHOS-r2.md`。使用者決定：自動鎖定時 shell 有輸出算活動；開 shell 不另外確認；自動鎖定預設啟用 15 分鐘（只影響沒設定過的人）；多行貼上不確認。
- 中風險 1：鎖定（含自動鎖定）時 `end_shells` 結束 shell 並記 `shell_close`，再清空終端機的操作分頁與 shell。
- 中風險 2：core 每個 shell 未畫出的輸出超過 1 MiB 就停止讀取 channel（SSH 流量控制擋住遠端），UI 以 `Shells::drawn` 回報；`process_events` 每輪最多 512 個事件（shell 輸出每 4 KiB 算一個），其餘以 1 ms 計時器接續。
- 中風險 3：有 shell 開著時關閉視窗會詢問（`CloseState.shells-only` 用不同文字）；shell 輸出重設閒置時間；關閉時 `end_shells` 記稽核。
- 中風險 4：`vt100` 的 `process` 與畫面轉換以 `catch_unwind` 包住，出錯時清空該 shell 的模擬器、標為失敗並關閉。
- 低風險：開啟結果改用 shell 編號比對；worker 崩潰時 shell 標為失敗；開啟中且已不忙碌的分頁可按 × 移除；只有連上過的 shell 才記 `shell_close`；AltGr（Ctrl+Alt）輸入的符號照字元送出；寫入卡住時仍可關閉 shell。
- 文件：`architecture.md` 的限制段落與自動鎖定說明、使用指南、PROJECT-DEV-ARCH。
- 未處理：即時輸出的單行長度上限（`console::Lines` 最長 32 MiB，拆行有遮蔽密碼的風險，需另外討論）、短密碼遮蔽（第一輪已知）。
- 測試：core 單元測試（`drawn`）、整合測試（3 MiB 輸出在畫面未回報時停在視窗附近，回報後全部送達）、app 單元測試（AltGr）、UI 測試（大量輸出分段處理並重設閒置時間、有 shell 時關閉會詢問、鎖定時結束 shell、記稽核並清空終端機）、偏好設定測試（沒有鍵時預設 15 分鐘、null 維持關閉、沒有設定檔時 15 分鐘）。`cargo test --workspace` 通過。

## 2026-11-15（終端機互動 shell，分支 `feat/WYSIWYG-Terminal`，基於 `be447e4`）

- 使用者要求：終端機可以互動輸入，需能使用 nano（完整終端機模擬，方案 B）；操作進行中使用者不能輸入。使用者同意嘗試，困難時再討論。
- 新增依賴：`vt100` 0.18.1（MIT，連帶 `vte` 0.15.0，MIT）。`cargo deny check licenses` 通過。
- core：新增 `shell.rs`（`Shells`、`Input`、`Screen`、`open`、`pump`）。`StorageSession` 處理 `shell_open`，`vault_lock` 先關閉所有 shell；稽核新增 `shell_open`／`shell_close`；debug 模式只允許 loopback。`open` 會等 PTY 與 shell 的回覆（russh 的 `request_pty`／`request_shell` 不等回覆）。
- app：worker 轉送 `ShellOutput`／`ShellEnded`；新增 `remote_shell.rs`（vt100 畫面轉樣式片段、按鍵轉位元組、貼上）；終端機面板加「開啟 Shell」、shell 分頁（>_、關閉鈕）、`ShellScreen`（格線、游標、焦點、滾輪、Ctrl+Shift+V）。忙碌或鎖定時拒絕輸入並顯示說明；結束時保留分頁與說明，提醒重新檢查主機；關閉 MISF 時關閉所有 shell。
- 測試：core 單元測試（`Shells` 關閉與編號）、整合測試（模擬 SSH 伺服器：PTY 類型與大小、回顯、視窗大小、遠端 `exit` 與本機 `close_all`）；app 單元測試（按鍵、貼上、畫面片段與寬字元、歷史捲動）與 UI 測試（開啟請求、輸出畫面與游標、按鍵與縮放送出、忙碌時暫停、關閉與移除分頁）。`cargo test --workspace`、兩種 clippy、`cargo fmt` 通過。
- 截圖：`MISF_OPEN_TERMINAL=shell` 的類 nano 畫面（反白標題列、綠色中文註解、寬字元對齊、256 色、背景色、游標）正確。
- 未驗證（需真實主機）：實際 nano／top／less 操作、按鍵手感與延遲、大量輸出效能、Ctrl+Shift+V 貼上是否可用、Windows 上 Tab 鍵是否被焦點切換攔截、中文輸入法（已知不支援）、自動鎖定時 shell 的結束。

## 2026-11-15（狀態列不再開關過往操作，分支 `feat/WYSIWYG-Terminal`，基於 `a2cdd94`）

- 使用者要求：狀態列的點擊開關與箭頭刪除，改由左側直條負責。
- `StatusBar` 只剩轉圈與最近一次結果；移除 `Shell.history-open`、`Shell.history-toggled`，以及標題列移除主機庫資訊後已無人使用的 `Shell.path`。
- 測試：過往操作的 UI 測試改經直條的時鐘按鈕（同一個無障礙名稱）開關。`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo clippy -p misf --all-targets --features capture -- -D warnings`、`cargo fmt` 通過。

## 2026-11-15（標題列命令改為選單樣式、主機庫資訊移到關於，分支 `feat/WYSIWYG-Terminal`，基於 `da58053`）

- 使用者要求：標題列右側的主機庫資訊（加密方式與路徑）移到「關於」；標題列的按鈕不要像按鈕，參考 JetBrains 的選單列。
- 新增 `MenuItem`（純文字、懸停與按下時反白、停用時變灰），取代標題列的 `Button` 與分隔線。「關於」頁新增「主機庫」「主機庫位置」兩列（路徑以 `char-wrap` 換行）。新增翻譯 `settings.storage`、`settings.storage_path`。
- 測試：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --check` 通過。
- 截圖：主視窗標題列只剩圖示與文字命令、右側只有視窗按鈕；「關於」頁的長路徑正常換行。
- 未驗證：懸停反白的實際觀感。

## 2026-11-15（自繪標題列，分支 `feat/WYSIWYG-Terminal`，基於 `ca94acc`）

- 使用者要求：做成 JetBrains 那樣的標題列。使用者已知並接受代價（Windows 11 最大化按鈕上的貼齊版面選單、系統標題列右鍵選單會消失，縮放改為自繪把手），選擇把現有頂端按鈕併入標題列。
- 主視窗改為 `no-frame`。新增 `title_bar.slint`：`WindowChrome`、`DragArea`（按住後第一次移動才交給 `drag_window`，因此雙擊仍可最大化／還原）、`CaptionButtons`、`WindowEdges`（5px 邊、10px 角，交給 `drag_resize_window`；最大化時隱藏，平時畫 1px 外框）。關閉按鈕送出 `CloseRequested`，與系統關閉走同一條路，操作進行中仍會詢問。最大化狀態由 winit 的 `Resized` 事件同步。沒有新增依賴（使用既有的 `unstable-winit-030`）。
- 測試：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo clippy -p misf --all-targets --features capture -- -D warnings`、`cargo fmt` 通過。
- 截圖（1280×820）：標題列一列，左側圖示與按鈕、右側主機庫資訊與三顆視窗按鈕，外框正常。
- 未驗證（截圖無法驗證，需實機操作）：拖曳移動、拖到螢幕邊緣貼齊、四邊四角縮放、雙擊最大化、最大化後是否蓋住工作列、Win+方向鍵、按鈕懸停提示、多螢幕與不同縮放比例。

## 2026-11-15（工具視窗開啟時頁面可捲動，分支 `feat/WYSIWYG-Terminal`，基於 `caa6389`）

- 使用者回報：工具視窗開啟時，主機詳情與批次作業頁的下半部被裁掉且無法捲動。原因是頁面有最小高度，外層 `clip` 只裁切不縮小。
- 主機頁與批次頁改包在 `ScrollView` 內（內容高度＝max(可見高度, 頁面最小高度)，頁面固定在捲動區原點並留 14px 給捲軸；工作區右側內距改為 14px 以維持原本外觀）。
- 測試：`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --check` 通過。
- 截圖（1280×820）：主機頁＋終端機、批次頁＋終端機出現捲軸；未開啟工具視窗時與原本相同、沒有捲軸。
- 未驗證：實際以滑鼠滾輪捲動外層頁面，以及與內層清單（套件表、批次表）滾輪的互動。

## 2026-11-15（IDE 式工具列：終端機與過往操作，分支 `feat/WYSIWYG-Terminal`，基於 `76a2e60`）

- 使用者要求：終端機與「過往操作」（使用者稱稽核紀錄，確認是指狀態列的過往操作）做成 JetBrains 左側圖示直條的樣式。使用者選擇：直條在視窗最左側、兩者共用頁面下方同一區塊且一次一個、終端機圖示一直顯示（沒有內容時顯示提示）。
- 新增 `tool_window.slint`（`ToolWindow`、`ToolStripe`、`DockHandle`、`DockTitle`）。`App::history_open` 與終端機自己的開關合併為 `App::tool: Option<Tool>`；操作開始時切到終端機，狀態列點擊開關過往操作。過往操作從浮在頁面上改為停靠在頁面下方。偏好設定的高度改為共用的 `tool_height`（`terminal_height` 只存在於這個未合併的分支，不做轉換）。
- 測試：UI 測試改為檢查 `ToolWindow.open`（互斥、再點收合、操作開始切到終端機、狀態列切換過往操作）。`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt` 通過。
- 截圖：`MISF_OPEN_TERMINAL=1`（批次頁＋終端機）與 `MISF_OPEN_HISTORY=1`（主機頁＋過往操作）在 1280×820 下正常；未開啟工具視窗時直條只有兩個圖示。
- 未驗證：滑鼠停留在圖示上的提示、實際拖曳高度的手感。

## 2026-11-15（底部即時終端機面板，分支 `feat/WYSIWYG-Terminal`，基於 `47a9b18`）

- 使用者要求：執行過程在下方顯示模擬終端機，參考 JetBrains IDE 的佈局。使用者選擇：即時遠端輸出（唯讀）、批次每台一個分頁、操作開始自動展開且結束後保留（可收合、可拖曳高度並記住）、顯示實際下的遠端命令。
- 核心：新增 `console.rs`（`Console`／`Echo`／`Lines`）。`ssh::execute` 收到資料時逐行送出，收到換行才送整行，避免切開 ANSI 序列、UTF-8 字元與密碼；每行以與 transcript 相同方式清理並移除 sudo 提示標記，再遮蔽該主機的密碼、sudo 密碼、私鑰密語與私鑰各行。單台檢查／升級／dist-upgrade／重新開機與批次的每台主機都會顯示；測試連線與 CLI 不顯示（`Silent`）。
- app：worker 的 `Screen` 把命令與輸出轉成 `Event::Terminal`，大量輸出以 `Worker::terminal` 旗標合併成一次喚醒。`ui/terminal.rs` 保存每台主機的分頁與行（上限 10,000 行，超過時一次捨棄最舊 2000 行），只對同一個 `VecModel` 追加新行；`terminal.slint` 是面板（固定行高 `ListView`、不換行、捲到底時跟隨）。
- 佈局：面板放在側欄與工作區下方、狀態列上方。第一次截圖時頁面最小高度把面板與狀態列擠出視窗，改為把側欄與工作區包進 `clip` 的 Rectangle（`min-height: 200px`），面板展開時頁面被裁切底部。面板高度若綁定視窗高度會造成 Slint 佈局綁定迴圈警告，改為只在拖曳與視窗縮放時夾住（頁面至少保留 420px）。
- 測試：`console::tests`（跨區塊的密碼、UTF-8、ANSI、sudo 標記與最後未換行的一行）；整合測試 `batch_reports_each_host_and_an_abort_skips_hosts_not_started` 加上假 SSH 伺服器的即時輸出檢查（每台有自己的命令與輸出、無 sudo 密碼、輸出中無提示標記、無 `\r`／ESC，未開始的主機沒有輸出）；UI 測試 `the_terminal_shows_each_hosts_output_live_in_its_own_tab`（分頁順序與去重、狀態、追加不重建、略過、複製、收合、上限）。`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo clippy -p misf --features capture -- -D warnings`、`cargo fmt` 通過。
- 截圖：`--features capture` 搭配新的 `MISF_OPEN_TERMINAL=1`，主機頁與批次頁在 1280×820 下面板、分頁與狀態列都正常顯示。
- 未驗證：真實主機上的即時輸出（apt 的進度行、PTY 下的換行）、長時間大量輸出時的介面流暢度、拖曳調整高度的手感。

## 2026-11-14（worker 崩潰時把結果未知寫入主機庫並列出主機，分支 `main`，基於 `0b3bb89`）

- 使用者指出前一版的「結果未知」只存在記憶體，而錯誤要求重新啟動，重啟後標示就消失；對話框也沒列出主機。依使用者指示兩項都做。
- 判斷移到 worker：`Running`（取代 UI 的 `App::pending`）記錄請求的 action／operation／id／ids，`Relay` 記錄批次中已回報的主機。panic 後若是升級，`mark_interrupted` 以 `Session::save_results` 把尚未回報的主機存為 `batch::interrupted`（存檔本身再包一層 `catch_unwind`），`Event::Crashed` 帶回主機 ID、結果與存檔錯誤。
- UI 的 `worker_crashed` 用這份清單標示主機（錯誤文字改用目前語系），錯誤訊息逐行列出主機名稱，存檔失敗時加一段說明重啟後會遺失標示。執行緒無聲結束時只解除忙碌並顯示錯誤（實際只會發生在啟動時，沒有進行中的升級）。
- 測試：新增 `worker::tests::a_crashed_upgrade_saves_its_unreported_hosts_as_unknown`（實際寫入 LUKSbox 後讀回；鎖定時回報存檔失敗）；UI 測試改為 `a_crashed_worker_frees_the_window_and_names_the_upgrades_left_unknown`。`cargo test --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt` 通過。
- 未驗證：真實 panic 的完整流程（沒有正常操作能觸發）。

## 2026-11-14（修正 MYTHOS 檢查報告的六項，分支 `main`，基於 `65542ce`）

- 來源：`.daybrake/dangerzone/MYTHOS.md`（未納入 Git）。使用者指定修 1–6 項，其餘不做；第 3、5、1 項的細節經使用者選擇（維持 1024 MiB、worker panic 後停止並要求重啟、任何操作進行中關閉都詢問）。
- 1. 操作進行中關閉主視窗：`on_close_requested` 在 `busy` 時保留視窗並顯示 `CloseDialog`（「繼續等待」／「仍要關閉」），操作結束後詢問自動消失。遠端脫離 SSH 執行等根本解法依使用者指示留待日後。
- 2. `plaintext_file` 在 `hosts.db`、`hosts.json` 之後依版本由小到大列出 `hosts.v<N>.bak.db`，沿用逐一確認路徑後刪除的流程；檔名須完全符合 `<stem>.v<數字>.bak.db`。
- 3. 主機庫上限 64 → 1024 MiB，單一命令輸出 2 → 32 MiB；使用指南新增「容量上限」。改上限前先實測：容器 metadata 區固定 1 MiB，暫時加入的 release 測試顯示單一檔案可到約 1.49 GiB（改寫三次後約 1.36 GiB），1 GiB 檔案連續改寫 10 次皆成功，每次寫入約 4.3–8.9 秒、讀取約 1.6–2.1 秒；量測用的測試已移除。
- 4. 批次開始時 `Session::handle` 改用 `hosts_for_audit` 一次讀取取得所有稽核主機（原本每台各解密一次）；每台結果仍在完成時各自寫入。
- 5. worker 以 `catch_unwind` 包住每個請求，panic 時送出 `Event::Crashed` 後結束執行緒；UI 的 `worker_crashed` 解除忙碌、顯示錯誤（要求重啟），並把中斷的單台升級／dist-upgrade 或批次中尚未回報的主機標示為「升級結果未知」。執行緒無聲結束（通道斷線）而仍忙碌時也同樣處理。`App::pending` 只記錄請求的 action／operation／id／ids，不保留含密碼的原始請求。
- 6. 撤銷金鑰槽時，若該槽是最後一組密碼（`Passphrase`），對話框多一段紅色警告，說明之後只剩 Passkey 的風險。
- 新增測試：`schema_upgrade_backups_are_offered_one_by_one_after_the_database`（core）、`closing_while_busy_asks_first_and_closes_at_once_otherwise`、`a_crashed_worker_frees_the_window_and_marks_interrupted_upgrades_unknown`、`revoking_the_last_password_warns_that_only_passkeys_would_remain`（UI）。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、24、59、24 項）。
- 未驗證：關閉詢問與撤銷警告的實際畫面（截圖只在閒置時拍攝，無法拍到忙碌中的關閉詢問）；接近 1 GiB 主機庫時完整 app 的實際速度與記憶體用量；真正的 worker panic（目前沒有已知路徑會觸發）。

## 2026-11-13（套用 Dependabot PR #6 uuid 1.29.2，分支 `main`，基於 `39fd69d`）

- 依 `docs/update-dependency.md` 處理。PR 的 CI 只因授權清單過期而失敗。uuid 1.29.2 只新增 wasm64 支援，不影響本專案。
- `cargo update -p uuid --precise 1.29.2` 又連帶把 bindgen 的 syn 3.0.9 改成 2.0.119、tempfile 的 getrandom 0.4.3 改成 0.3.4（與 PR #4／#5 時相同）；手動改回後，鎖檔差異與 PR 完全相同，`cargo metadata --locked` 通過。
- `./licenses.ps1` 重新產生；`cargo deny --locked check` 全部通過。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、21、58、24 項）。

## 2026-11-13（程式內顯示與 Release 相同的版本名稱，分支 `main`，基於 `825ec3b`）

- 使用者要求程式內也顯示 `dev-27.1-825ec3b` 這類名稱。`app/build.rs` 新增 `build_name()`：在 GitHub Actions 中，推送 `v*` 標籤時用標籤名稱，否則用 `dev-<run_number>.<run_attempt>-<短 commit>`，與發布步驟的規則相同；本機建置為 `v<Cargo 版本>`。以 `MISF_BUILD_NAME` 提供給設定頁「關於」與 LOG 匯出標頭。workflow 發布步驟加註解，提醒兩處要一起改。
- 使用者決定：不處理「只重跑失敗工作」時 run_attempt 與執行檔差一號的情況（不採用這種重跑）；本機建置不另外處理。
- 驗證：在本機設定假的 GitHub Actions 環境變數建置 debug 版，執行檔內含 `dev-99.1-abcdef1`；改為標籤 `v0.1.0` 時含標籤名稱；移除變數後會重新編譯，不再含 `dev-99`（`rerun-if-env-changed` 有效）。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、21、58、24 項）。
- 未驗證：真正的 CI 建置顯示的名稱是否與 Release 相同（要等下次推送後的 pre-release）。Windows 執行檔內容的產品版本未改。

## 2026-11-12（新增更新依賴的操作文件，分支 `main`，基於 `b64f596`）

- 使用者要求寫 `update-dependency.md`，說明更新依賴該做什麼。新增 `docs/update-dependency.md`：需要的工具、Dependabot 的行為（PR 只對 `main`、不會重新產生授權清單所以 CI 必定失敗）、套用 PR 的十個步驟（含鎖檔比對與手動改回連帶改動）、手動升級、特定套件的額外工作、CI 因安全公告失敗時的處理。
- `docs/dependency-updates.md` 開頭連到新文件，並修正「並並」錯字；`docs/PROJECT-DEV.md` 索引加入新文件。
- 只改文件，未改程式碼，未重新執行 cargo 檢查。

## 2026-11-12（套用 Dependabot PR #4 rfd 0.18.0、#5 uuid 1.29.1，分支 `main`，基於 `6263787`）

- 兩個 PR 的 CI 都只因 `licenses/THIRD_PARTY_LICENSES.html` 未重新產生而失敗（Dependabot 不會執行 `./licenses.ps1`），程式本身沒有問題。
- 在本機 `main` 套用相同改動：`app/Cargo.toml` 的 rfd 改為 0.18.0；鎖檔連帶加入 objc2-uniform-type-identifiers 0.3.2（macOS）、pollster 0.4.0 → 1.0.1。rfd 0.18 只新增功能與修正（MSRV 1.92，本機 1.99.1），本專案用到的 `FileDialog`／`MessageDialog` API 未變。
- `cargo update -p` 又連帶把 bindgen 的 syn 3.0.9 改成 2.0.119、tempfile 的 getrandom 0.4.3 改成 0.3.4，造成 bindgen 編譯失敗；手動改回後，鎖檔改動與兩個 PR 完全相同，`cargo metadata --locked` 通過。
- `./licenses.ps1` 重新產生；`cargo deny --locked check` 全部通過。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、21、58、24 項）。
- 未驗證：在執行中的 app 開啟檔案對話框（匯入／匯出、LOG 匯出）；macOS 與 Linux 只靠 CI。

## 2026-11-11（狀態列可展開過往操作，分支 `main`，基於 `2c2bc68`）

- 使用者要求在底部狀態列加一個「>」，可展開顯示過往操作。
- 記錄方式：原本散在各處的 `status = …` 改為 `set_status`；針對目前主機的結果（檢查／升級、連線測試、指紋）用 `set_host_status`，紀錄會加上主機名稱（語系字串 `status.host_line`）。`note_error` 記錄錯誤本文並標為錯誤。「處理中」、進度訊息與通用的「操作失敗」直接寫入，不列入紀錄。
- 紀錄只存在記憶體，最多 200 筆（`HISTORY_LIMIT`）；`clear_host_cache` 會清空，所以鎖定或自動鎖定後不留主機名稱。
- UI：`StatusBar` 整列可點，左側箭頭收合時朝右、展開時朝上。展開時 `HistoryPanel` 浮在狀態列上方、疊在頁面上，由新到舊、最多約 200px 後捲動；錯誤用錯誤色。一開始把清單放進版面裡，結果壓縮頁面，甚至把狀態列擠出視窗，所以改為浮動。
- 新增測試 `status_history_keeps_results_and_errors_but_not_progress`：進度與「處理中」不記錄；檢查結果帶主機名稱；錯誤標記正確；狀態列的無障礙動作可開關；Shell 清單由新到舊；`vault_lock` 後只剩「已鎖定」。
- 截圖新增 `MISF_OPEN_HISTORY=1`，目視確認收合與展開。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、21、58、24 項）。
- 未在執行中的 app 實際操作：滑鼠游標、懸停變色、長錯誤訊息換行後的捲動。

## 2026-11-11（對話框按鈕掉出面板，分支 `main`，基於 `3917688`）

- 使用者回報：重新開機確認的「下一步／取消」掉到面板外。原因是 Slint 1.18.1 的回歸（slint-ui/slint#13655、#14863）：`if`／`for` 版面儲存格裡的換行文字，偏好高度以不換行計算，所以每多換一行，面板就少算一行。截圖也發現密碼庫建立與 Passkey 對話框最底下的 DEBUG 提示同樣掉出面板。
- 修正：`widgets.slint` 的 `Body`／`Notice`／`Heading`／`Mono` 加上 `min-height: self.preferred-height`（上游 issue 與其他專案採用的做法），對話框結構不變。先試過把重新開機對話框攤平成逐行 `if`，也有效，但只修一處，所以改回共用元件修正。
- 新增測試：`pages_respond_to_clicks` 在 1280 與 520 寬度下，確認重新開機對話框的「下一步」位於 `Modal::panel` 內。暫時移除修正後，測試失敗（按鈕底部 553.5、面板底部 533.5，與截圖一致）；放回修正後通過。
- 截圖新增 `MISF_OPEN_REBOOT=1`／`final`。截圖目視確認：重新開機兩步、密碼庫建立、Passkey、SSH 金鑰管理、主機頁、編輯主機、設定的「安全性」頁版面正常。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、20、58、24 項）。
- 未截圖：升級確認、dist-upgrade 報告、批次失敗、設定的其他頁（共用元件只增加最小高度，預期不會變矮）。

## 2026-11-10（CI 加入安全公告檢查，分支 `main`，基於 `aec5b64`）

- 使用者同意後，在 `build-prerelease.yml` 的 Linux 工作中，`cargo deny --locked check licenses` 之後加入 `cargo deny --locked check advisories`。步驟改名為「Check dependency licenses and security advisories」；任一失敗，都不會建置發布版本。
- 已知的 5 筆 unmaintained 例外沿用 `deny.toml`。`deny.toml` 註解、`docs/dependency-updates.md`（新公告的處理原則：先評估，有修補就升級，不直接加例外）、`docs/PROJECT-DEV.md`（發布條件）已同步。
- 本機以 cargo-deny 0.20.2（與 CI 相同版本）執行兩條指令：licenses ok、advisories ok。工作流程只以 diff 目視確認縮排（本機沒有 YAML 解析器）。CI 要推送後才會實際執行。

## 2026-11-10（核對 CI 與授權文件，分支 `main`，基於 `f2853f8`）

- CI：`build-prerelease.yml` 在 Slint 版 `3f7942d` 上兩個平台都成功（run 42822716899），並發布 `dev-22.1-3f7942d`。Linux 安裝套件涵蓋 Slint（winit、Skia、AccessKit D-Bus）與 libfido2 的需求。CI 只跑 `cargo deny check licenses`，沒有跑 advisories。
- 授權：`licenses.ps1 -Check` 通過；HTML 含 slint／i-slint-*、skia-*、russh 0.65.1 與 GPL-3.0 全文。skia-bindings 未啟用 textlayout，所以沒有 ICU／HarfBuzz（`misf.exe` 內找不到 ICU 資料）；未啟用 embed-freetype，Linux 使用系統 FreeType，與 `THIRD_PARTY_NOTICES.md` 一致。Noto Sans TC OFL 與 Skia 授權都會打包。
- 修正：`docs/PROJECT-DEV.md` 的 Linux 建置與執行需求改為與 CI 一致（xkbcommon、FreeType、D-Bus；rfd 在 Linux 使用 XDG Desktop Portal，不需要另外的對話框開發套件）；`platform-builds.md` 記錄 Linux CI 已通過。
- 只改文件，未重跑測試。

## 2026-11-10（cargo-deny 停止維護公告記為例外，分支 `main`，基於 `f884c7a`）

- 使用者同意把 5 筆 unmaintained 公告記為已知例外：`deny.toml` 新增 `[advisories] ignore`（RUSTSEC-2026-0441 pin-utils、0192 ttf-parser、0371 zbus_names、0370 zvariant、0372 zvariant_derive），每筆附原因。`cargo tree -i` 確認這些套件都只經由 Slint 1.18.1 引入（spin_on；winit sctk-adwaita；AccessKit AT-SPI）。`docs/dependency-updates.md` 新增「已知的停止維護公告」表格與移除條件。
- `cargo deny --locked check`：advisories、bans、licenses、sources 全部 ok（仍有 duplicate 與 LUKSbox git 來源的警告，未改）。
- 只改設定與文件，未改程式碼，所以沒有重跑 fmt／Clippy／測試，也沒有重新打包。
- 同日已關閉 Dependabot PR #2（目標分支 `legacy-egui`）。確認 `dependabot.yml` 沒有設定 `target-branch`，只會針對預設分支 `main`；CI 只在 push／PR 到 `main` 與 `v*` 標籤時觸發，`legacy-egui` 不會收到機器人 PR，也不會跑 CI。

## 2026-11-10（russh 0.65.0 → 0.65.1，分支 `main`，基於 `3f7942d`）

- 使用者轉來 Dependabot PR #2（目標分支是 `legacy-egui`，不是 `main`）。在 `main` 上套用相同的鎖檔改動：russh 升到 0.65.1，`generic-array 1.4.5` 不再依賴 `generic-array 0.14.7`。`core/Cargo.toml` 的版本需求 `0.65.0` 未改。
- `cargo update -p russh --precise 0.65.1` 會順帶把 `windows-sys`／`getrandom` 等套件解析成較舊的版本，超出範圍，所以還原後改為手動修改鎖檔，結果與 PR 相同。`cargo metadata --locked` 確認鎖檔一致；`rsa` 仍是 `0.10.0-rc.18`（見 `decisions/rsa-advisory.md`）。
- `./licenses.ps1` 重新產生授權清單與鎖檔雜湊（cargo deny licenses 通過）。
- fmt、Clippy（`--all-features --locked -D warnings`）通過；`cargo test --workspace --locked` 全部通過（22、20、58、24 項）。
- `cargo deny check` 的 advisories 失敗：pin-utils、ttf-parser、zbus_names、zvariant、zvariant_derive 已停止維護（RUSTSEC-2026-0441／0192／0371／0370／0372）。這些套件的鎖定版本這次都沒改，CI 也只跑 `check licenses`，與本次更新無關，未處理。
- 未實際連線 SSH 主機測試新版 russh。

## 2026-11-10（檢查時找出分階段延後套件，分支 `slint-rego`，基於 `e492be8`）

- 使用者要求在 APT 檢查階段就偵測 phasing 延後的套件。檢查有待更新套件時，於讀完清單後執行唯讀的 `LC_ALL=C apt-get -s -q=1 upgrade`（`apt::PREVIEW`，`Commands.preview` 只有 APT；root 與一般帳號都不經 sudo，與既有 `SIMULATE` 相同），以既有 `parse_phased` 解析；失敗或非零退出碼只讓 `phased` 為空。命令列入 `manager_commands(Check)`，提示與右鍵複製會顯示它。
- 推論：`apt-get -s` 的模擬輸出也含 kept back 與 `Inst` 行；原本載入時 `parse_plan` 不分操作掃整份輸出，檢查結果會被誤判成有 dist-upgrade 計畫。改為 kept back 與 plan 只從 upgrade／dist_upgrade 結果解析，`phased` 則所有結果都解析。
- UI：批次表格「分階段延後」欄改為任何成功結果都顯示數字與套件提示；主機頁新增一行提示（`phased.banner`）。
- 新增／更新測試：sqlite 往返確認檢查結果只讀回 phased、不產生 kept back 與 plan；整合測試的 APT 檢查回報 `phased == ["openssl"]`、無 kept back／plan，且實際執行的命令與確認清單一致；批次檢查測試排除 PREVIEW 後仍沒有任何升級命令；UI 測試確認主機頁提示與批次欄位。截圖目視確認主機頁提示與批次欄位。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、20、58、24 項）。
- 未在真實 Ubuntu 主機驗證：一般帳號執行 `apt-get -s upgrade` 時的 phasing 判斷是否與 root 相同（phasing 依 `/etc/machine-id`，一般帳號可讀，推論應相同）。舊的檢查紀錄沒有模擬段落，下一次檢查前「分階段延後」欄會顯示 0。

## 2026-11-10（自繪右鍵選單，分支 `slint-rego`，基於 `a33ffad`）

- 使用者要求右鍵選單與介面風格統一。Slint 內建 `ContextMenuArea` 的彈出選單依 widget 樣式（fluent `FluentPalette`）繪製，顏色為 `out` 屬性，只能切換 `color-scheme`，無法套用 Theme；官方文件（docs.slint.dev ContextMenuArea、TouchArea 頁）也沒有外觀設定。改為自繪：`MenuLine`／`PopMenu`／`MenuArea`／`MenuOverlay`（widgets.slint），外觀沿用進階選單（Theme.surface、border、6px 圓角、TextButton 項目），主視窗 `run-menu` 依 `action` 分派。
- 右鍵偵測：TouchArea 會抓住任何按鍵的按下，官方文件只提供 `enabled: false` 讓事件穿過。`MenuArea` 疊在內容上方，只在 `PopMenu.right-down` 為真時啟用；主視窗 winit 監看（`track_right_button`）在 Slint 處理前於右鍵按下時設為真，其他按鍵按下、游標離開、失焦一律清除，避免旗標卡住吞掉左鍵。
- 一併修正：批次表格主機欄與側欄分類標題的右鍵選單也是包在 TouchArea 外，原本同樣收不到右鍵（未曾在執行中的 app 驗證過），現改用 `HintArea.menu`。
- 新增測試（以實際指標事件，測試後端沒有 winit 監看，由測試設定 `right-down`）：主機頁與進階選單內右鍵複製內容等於提示命令；點選單外只關閉右鍵選單、不複製、進階選單仍開啟；批次表格右鍵開明細視窗與前往主機頁；側欄分類右鍵重新命名開啟對話框。
- 截圖新增 `MISF_OPEN_MENU=1`（搭配 `MISF_FIXTURE=1`）。深色中文與英文截圖目視確認右鍵選單樣式與進階選單一致；進階選單位置在截圖中以固定座標模擬。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、20、57、24 項）。
- 未在執行中的 app 實際操作：winit 監看設定旗標的時機（理論上監看先於 Slint 處理）、系統剪貼簿收到文字、選單靠近視窗邊緣時的翻轉位置。

## 2026-11-10（文件對齊 Slint 版現況，分支 `slint-rego`，基於 `f6bbbd4`）

- 只改文件，未改程式碼。逐份對照程式碼修正：進階選單不支援 Esc 關閉（使用指南原寫可以）、字型「系統字型」說明、閒置自動鎖定已存在（PROJECT-DEV、LUKSbox 設計原寫尚未提供）、請求協定與批次補上 `dist_upgrade`、`ui-settings.json` 含字型、Clippy 指令加上 `--all-features`、UI 測試說明、`app/build.rs` 與 `widgets.slint` 職責。`passkey-testing.md`、`dependency-updates.md` 核對後內容仍正確，未改動也未改標頭。
- 未執行建置或測試（上一筆的檢查結果仍適用）。

## 2026-11-10（右鍵複製命令修正，分支 `slint-rego`，基於 `3d90ce4`）

- 使用者回報進階選單內的右鍵複製失效。實際上 Slint 版所有 `HintArea` 的右鍵複製都失效：`ContextMenuArea` 包在控制項外層，但 Slint 的 `TouchArea` 對任何按鍵的按下都回傳 `GrabMouse`，事件分派在子項抓取後就停止（`i-slint-core` `input.rs` `send_mouse_event_to_item`），外層 `ContextMenu::input_event` 收不到右鍵。改為把 `ContextMenuArea` 疊在控制項上方：它只接受右鍵按下，其他事件回傳 `EventIgnored` 交給下方按鈕，左鍵與懸停提示不受影響。
- 新增測試：以實際右鍵事件點主機頁「測試連線」與進階選單「APT 升級…」，確認出現複製選單，以方向鍵＋Enter 選擇後剪貼簿內容等於提示的命令，且進階選單保持開啟。暫時還原修正時測試在「沒有複製選單」處失敗，確認能抓到此錯誤。測試後端的選單項目座標是彈出視窗內座標，因此以鍵盤選擇，不以點擊。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、20、57、24 項）。
- 未在執行中的 app 實際確認系統剪貼簿收到文字（`ClipboardSink` 以隱藏 `TextInput.copy()` 寫入，測試後端只能驗證 `Clip` 內容）。

## 2026-11-10（補齊 Skia 內含元件授權，分支 `slint-rego`，基於 `cc7ba2e`）

- 依 rust-skia 的建置參數與 Windows `skia.lib` 符號確認，預編譯 Skia 內含 libpng、libjpeg-turbo、zlib、Expat、Wuffs、D3D12 Memory Allocator；FreeType、HarfBuzz、ICU、libwebp 未編入。依 Skia `m153-0.101.2` 的 `DEPS` 釘選版本，自上游 googlesource 取得各授權全文放入 `licenses/Skia/third_party/`（Wuffs 無 NOTICE）；Linux 版另依 DEPS 附 Vulkan Memory Allocator 授權，但未實際建置 Linux 版驗證。
- `THIRD_PARTY_NOTICES.md` 加入元件表與 IJG 要求的致謝句；打包改為複製整個 `licenses/Skia/`。

## 2026-11-10（改用 Skia、批次欄寬拖曳修正，分支 `slint-rego`，基於 `2e87553`）

- 使用者比較後選定 Skia：繪圖引擎固定為 Skia，移除 femtovg 與 `skia` 建置選項。授權清單重新產生（含 skia-safe／skia-bindings），另附 Skia 本身的 BSD-3 全文 `licenses/Skia/LICENSE`，聲明檔列出預編譯檔內含的 libjpeg-turbo、libpng、zlib、Wuffs 等元件（各元件授權全文尚未逐一附上）。
- 使用者回報批次作業拉動不見：欄寬拖曳把手外的提示元件寬高為 0（先前把提示元件改為不伸展後暴露），補上尺寸；另外每次拖動都整個替換欄位清單，Slint 重建把手元件，拖動只生效一次且放開不存檔，改為就地更新。新增測試：以實際指標事件拖曳欄寬邊界，權重改變並在放開時存檔。
- 字型選項「系統字型」不再附「（原本的字型）」。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過。

## 2026-11-10（字型選項、Skia 比較版、提示位置修正，分支 `slint-rego`，基於 `62da7a9`）

- 使用者回報兩處跑版：主機頁「刪除」的提示區域把整列剩餘寬度吃掉（提示元件預設改為不橫向伸展），以及提示框在右下角超出視窗（覆蓋層沒有指定位置，被置中擺放而整體偏移約 100 px；改為鋪滿視窗並把提示框限制在視窗內）。新增測試：提示放在右下角時提示框仍完整在視窗內。
- 新增偏好設定「字型」：Noto Sans TC、Noto Sans TC 粗體、系統字型（原本的字型），兩個視窗同時套用；偏好設定往返與預設值有測試，三種選項截圖確認。
- 使用者回報字型霧霧的：femtovg 只做灰階反鋸齒、沒有字型微調。依使用者要求另建 Skia 版（`--features skia`）供並排比較，截圖上 Skia 的筆畫明顯較清楚。DPI 方面 winit 已設為每螢幕 DPI 感知，不是點陣放大造成。
- fmt、Clippy（`--all-features -D warnings`，含 Skia）通過；`cargo test --workspace` 全部通過（22、20、57、24 項）。

## 2026-11-10（Slint 版補完：Noto Sans TC、授權、Esc、互動測試，分支 `slint-rego`，基於 `a20ea4e`）

- 介面字型改為使用者提供的 Noto Sans TC 可變字重字型，內嵌於執行檔（約多 12 MB）；OFL 授權放在 `assets/fonts/OFL.txt`，打包時附為 `licenses/NotoSansTC/OFL.txt`。側欄卡片高度配合新字型的行高調整。
- 授權：Slint 依 GPL-3.0-only 使用，`deny.toml`／`about.toml` 加入 GPL-3.0-only，移除已不再使用的 OFL-1.1、Ubuntu-font-1.0 套件授權；刪除 `licenses/egui-fonts/`，重新產生 `THIRD_PARTY_LICENSES.html`；`THIRD_PARTY_NOTICES.md` 改寫 egui 段落為 Slint 與 Noto Sans TC。打包時先清空輸出的 `licenses/`，避免舊的 egui 字型授權殘留。
- 批次套件明細視窗與「需要 dist-upgrade」報告可按 Esc 關閉（與 egui 版相同，其他對話框不受 Esc 影響）。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、20、57、24 項）。Slint 測試後端以實際指標與鍵盤事件驗證：側欄拖曳排序送出新順序、懸停 1 秒不出現而 1.7 秒出現提示且移開即消失、Esc 關閉明細視窗、批次表格雙擊、進階選單、設定樹。
- 截圖確認新字型的主機頁、批次頁、緊湊側欄與設定視窗。仍未在執行中的 app 實際等待閒置自動鎖定（winit 輸入監看與每秒檢查無法在測試後端執行；計時邏輯有單元測試），右鍵複製只驗證了複製內容，未實際開啟右鍵選單。

## 2026-11-10（UI 改用 Slint，分支 `slint-rego`，基於 `1c6f0cf`）

- 移除 eframe／egui，畫面改以 Slint 1.18.1（GPLv3 授權選項、fluent 樣式、winit＋femtovg）宣告於 `app/ui/*.slint`；原 6300 行的 `ui.rs` 拆成 `app/src/ui/` 下的狀態模組與各頁模組，最大檔約 900 行。設定改為獨立系統視窗。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、20、57、24 項）。原 egui 畫面測試改寫為狀態邏輯測試，另以 `i-slint-backend-testing` 點擊實際元件：進階選單開啟與選項、批次表格雙擊切到主機頁、設定樹切換與收合。新增主機庫密碼檢查測試時發現並修正：上一次的錯誤訊息會殘留，讓修正後的密碼仍被擋下。
- 以 `--features capture` 加 `MISF_FIXTURE=1` 假資料截圖目視確認：主機頁、批次頁、設定視窗（深色中文、淺色英文）、主機編輯與 SSH 金鑰對話框、建立主機庫對話框。批次說明原本提到已移除的「表格左上角 ⇅」，改為指向設定頁。
- 尚未完成或未驗證：未在執行中的 app 實際操作拖曳排序、懸停提示時機、右鍵複製命令、閒置自動鎖定與視窗大小記憶；批次明細視窗不支援 Esc 關閉；打包腳本與第三方授權（移除 egui 字型、加入 Slint）尚未更新，此分支在功能對齊前不打包。

## 2026-11-09（設定視窗分類樹，基於 `69cd0ec`）

- 設定左側改為分類樹：「偏好設定」可收合，底下分為外觀與主機清單、安全性、批次作業；右側標題顯示「偏好設定 › 頁名」，各頁以分隔線區段分組。`MISF_OPEN_SETTINGS` 新增 `security`、`batch`。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、18、57、24 項）。新增 UI 測試：子頁縮排、點子頁切換與標題路徑、分支收合與展開、從其他頁點分支開啟第一頁。
- 以 `--features capture` 截圖目視確認安全性與批次作業頁的樹狀導覽與區段分隔線（測試資料夾無主機庫，畫面上同時有建立主機庫對話框）。

## 2026-11-09（閒置自動鎖定，基於 `ee69d1b`）

- 設定 > 偏好設定新增「自動鎖定」：永不（預設）或閒置 1～1440 分鐘後鎖定。只計使用者輸入；作業執行中不鎖定。時間到先清除主機快取與升級／重開機確認、dist-upgrade 報告等視窗，再送出 `vault_lock`，稽核為 `vault_auto_lock`。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、17、57、24 項）。新增：自動鎖定計時、輸入與忙碌重新計時、到時清除並送出鎖定、未加密不動作；偏好設定往返與超出範圍值視為永不。
- 視窗最小化後還原時，先判斷是否超時再處理輸入（有單元測試）；未在執行中的 app 實際等待閒置鎖定，也未確認最小化期間 eframe 是否仍會依排程重繪。

## 2026-11-09（右鍵複製命令、進階操作選單、批次雙擊跳主機頁，基於 `10ff78c`）

- 主機頁的升級、dist-upgrade、重新開機收進「進階操作」選單；選單開關由 App 自行保存，項目的滑鼠提示與右鍵複製命令仍可使用。批次表格雙擊主機名稱切到主機頁，右鍵選單提供「前往主機頁面」與（視窗模式）「顯示套件明細」。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、16、57、24 項）。新增 UI 測試：選單內右鍵複製到剪貼簿且選單不關閉、左鍵選項目開啟確認並關閉選單、點外面關閉；批次右鍵開啟明細視窗、雙擊切到主機頁。
- 未在執行中的 app 目視確認選單外觀。後續：⏷（U+23F7）在載入的字型中沒有字形而顯示為方框，已改為以 `Atom::paint` 繪製三角形（開啟時朝上）。

## 2026-11-09（kept back 報告、dist-upgrade、sudo 重啟狀態、輸出標題框，基於 `6519027`）

- APT upgrade 的 kept back 套件存入 `kept_back`，並以唯讀 `apt-get -s dist-upgrade` 模擬變動；操作結束時 UI 彈出報告。新增 `dist_upgrade`（單台與批次）。重啟狀態一般帳號改為整段 `sh -c` 放進 sudo。執行輸出與 LOG 改用依顯示寬度對齊的星號框標題。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、14、57、24 項）。新增或更新：kept back 與模擬計畫解析、sudo upgrade 觸發模擬且命令順序與確認清單一致、dist-upgrade 只執行自己的命令且非 APT 不碰套件、一般帳號經 sudo 讀重啟狀態、UI 批次結束產生報告、LOG 框線格式。
- 未在執行中的 app 目視確認報告視窗與按鈕；未在真實主機執行 dist-upgrade 或驗證 sudo-rs 下的狀態命令。

## 2026-11-09（完整版 LOG 與分段格式，基於 `c5bfe1e`）

- LOG 改為完整內容：請求與回應全文，只遮蔽密碼、私鑰、密語、PIN 欄位；每次遠端操作有 `********` 標題，每台主機一個區塊到 `END OF <名稱>`，含 ID、帳號@位址、指紋與遠端原始輸出。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、14、56、23 項）。測試涵蓋：巢狀機密欄位遮蔽但 ID／位址／帳號保留、主機區塊格式、UI 中解鎖與批次流程的標題與區塊、匯出與 16 MiB 上限。
- 未在執行中的 app 目視確認；未用真實主機的批次升級輸出檢查匯出檔。

## 2026-11-09（記憶體 LOG 與匯出，基於 `c68612e`）

- 新增 `app/src/session_log.rs`，在「設定 > LOG」匯出；稽核新增 `log_export`。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、14、56、23 項）。新增：請求只保留非機密欄位、結果附輸出、超過上限捨棄最舊、匯出檔內容，以及 UI 層以解鎖密碼／主機密碼送入後 LOG 不含任何機密。
- 未在執行中的 app 目視確認 LOG 設定頁與存檔對話框。

## 2026-11-09（APT 重啟提示與重新開機，基於 `aeb97d1`）

- APT root 在檢查／升級後執行唯讀 `apt::STATUS`（reboot-required＋`needrestart -r l -b`）；一般帳號不執行。新增 `reboot` 請求（`shutdown -r +1`，只限 APT），UI 兩次確認。SQLite v6：`restart_status`、`restart_items`。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、10、56、23 項）。新增：`parse_restart` 解析、reboot 命令形式、root 升級後讀取重啟狀態且命令順序與確認清單一致、一般帳號不執行 needrestart、reboot 只跑一條固定命令且拒絕非 APT 主機、SQLite 往返含重啟狀態、舊設定檔預設顯示新欄位。
- 未在執行中的 app 目視確認「重啟提示」分頁、警示列與兩次確認視窗；未在真實 Ubuntu 主機驗證 `needrestart -b` 輸出格式與 `shutdown -r +1`。

## 2026-11-08（批次逐台儲存與失敗詢問，基於 `8d40cd3`）

- 新增 `core/src/batch.rs`：每台完成即儲存並回報；失敗時暫停開始新主機，等待繼續／中斷；中斷後未開始的主機列在 `skipped`，稽核記為 cancelled。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、10、54、21 項），新增 `batch_reports_each_host_and_an_abort_skips_hosts_not_started`（12 台、第一台指紋不符→中斷），連跑 10 次皆通過。
- 未在執行中的 app 目視確認失敗詢問視窗與表格逐列更新；skipped→cancelled 的稽核紀錄沒有專屬測試；未對真實主機實測。

## 2026-11-08（拆分 package_manager 模組，基於 `630cda6`）

- 純重構：`ssh.rs` 只留 SSH 傳輸；套件管理移到 `core/src/package_manager/`（apt／dnf／yum／pkg／rpm／detect／operation）。命令字串內容未改，只換位置。
- fmt、Clippy（`--all-features -D warnings`）通過；`cargo test --workspace` 全部通過（22、10、54、20 項），新增 root／sudo 命令形式、ID 往返、套件管理器選擇與各管理器失敗判斷的單元測試。
- 未對真實主機重新實測。

## 2026-11-08（測試連線資訊，基於 `2669c10`）

- fmt、Clippy（`-D warnings`）通過；`cargo test --workspace` 全部通過（22、10、46、20 項）。
- 整合測試在 loopback 伺服器驗證：成功時記錄伺服器版本、kex、cipher、MAC、主機金鑰 `ssh-ed25519` 與指紋、密碼／公鑰登入；指紋不符與密碼錯誤時狀態為 failed、仍保留已協商資訊、未記錄登入方式，錯誤不含密碼。
- SQLite 往返測試涵蓋 `connection_tests`（v5）。
- 未在執行中的 app 目視確認「連線資訊」分頁排版；未對真實伺服器實測。

## 2026-11-08（移除 RSA，基於 `b63645b`）

- fmt、Clippy（`-D warnings`）通過；`cargo test --workspace` 全部通過（22、9、46、20 項），新增 `rsa_private_keys_are_rejected_in_both_formats`。
- `cargo audit`：退出碼 0，RUSTSEC-2023-0071 不再命中；`cargo tree -i rsa` 找不到套件。
- 未對只提供 RSA 主機金鑰的真實伺服器實測錯誤訊息。

## 2026-11-07（commit `af2c75e`，強制加密）

- `cargo fmt --all -- --check`：通過。
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`：通過。
- `cargo test --locked --workspace`：全部通過（各測試目標 22、9、45、20 項）。
- `.\package.ps1`：成功；`dist\MISF\misf.exe` 與 `target\release\misf.exe` 時間 2026-11-07 00:09:32、雜湊相同。
- 未在執行中的 app 目視確認新的建立加密主機庫畫面。

## 較早紀錄

- 2026-10-29 LUKSbox 整合時：完整工作區 19 項測試與 Clippy 通過（見 `luksbox-integration.md`）。
- 2026-10-30 依賴更新後：Windows 工作區 62 項測試通過（見 `dependency-update-2026-10-30.md`）。
