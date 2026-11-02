# MISF Management Terminal 開發說明

一般使用方式見 [README](../README.md)。本文件給開發者與維護者，記錄建置、儲存格式、安全設計細節與測試方式。

全 Rust 原生桌面客戶端，以 eframe／egui 繪製畫面，內建 Rust SSH 核心。畫面與核心編譯成同一個執行檔，不需要 Java、瀏覽器或管理伺服器。

## 結構

- `app/`：eframe／egui 原生 UI；背景執行緒執行網路及磁碟作業。
- `core/`：主機資料、SSH 及套件管理核心；亦提供 stdin/stdout JSON CLI。
- CLI 的加密工作階段只存在同一程序內；可透過 stdin 傳入 JSON 請求陣列，依序執行解鎖、操作與鎖定。不要把解鎖密碼放進命令列參數。
- [架構](architecture.md)：目前實作架構、資料表與 JSON 協定。

## 啟動與打包

開發需要 Rust 1.95 以上、Git 與該作業系統的原生建置工具（Windows 使用 MSVC）。

```powershell
.\run.ps1
```

建置與產生可攜式 Windows 執行檔：

```powershell
.\build.ps1
.\package.ps1
```

打包後直接執行 `dist/MISF/misf.exe`。

圖示素材放在 [`assets/icons/`](../assets/icons/)，已納入版本控制。Windows 執行檔與程式視窗均內嵌圖示；Linux 套件另附選單項目與各尺寸圖示，安裝方式見 [`assets/README.md`](../assets/README.md)。

Linux／macOS 可使用 `cargo run --locked -p misf`，須在對應平台建置；Linux 建置需要 OpenGL、X11／Wayland、fontconfig 與檔案對話框相關開發套件，Passkey 另需 `clang libclang-dev libfido2-dev`。此版本已在 Windows 建置與驗證。

## 自動建置與 Pre-release

[GitHub Actions](https://github.com/Sparxilium-Network/MISF-Management-Terminal/actions/workflows/build-prerelease.yml) 在 `main` 更新、推送 `v*` 標籤或手動執行時，自動測試並建置 **Linux x86_64** 與 **Windows x86_64**。兩個平台的格式、Clippy、測試與 release 建置全部通過後，才上傳套件並公開 **Pre-release**，固定不標記為 Latest。PR 只驗證與產生 Actions artifacts，不發布。

- Windows：`MISF-Management-Terminal-windows-x86_64.zip`。
- Linux：`MISF-Management-Terminal-linux-x86_64.tar.gz`。這是原生動態連結套件，需要圖形桌面及相容的 glibc、OpenGL、X11／Wayland、fontconfig 等系統函式庫；不是 AppImage，也不保證支援較舊發行版。
- 每個套件內附 README、`docs/`、本專案 AGPL-3.0 授權全文 `LICENSE`，以及第三方授權（`THIRD_PARTY_NOTICES.md`、`licenses/`：所有編入執行檔的 Rust 相依套件授權全文、egui 內嵌字型與 LUKSbox 授權）；另附 SHA-256 校驗檔。
- 相依套件授權清單 `licenses/THIRD_PARTY_LICENSES.html` 由 `./licenses.ps1` 以 cargo-about 依 `Cargo.lock` 產生，並先以 `cargo deny check licenses` 依 `deny.toml` 確認沒有與 AGPL 不相容的授權；更新相依後須重新執行並提交。清單與 `Cargo.lock` 不一致時，`package.ps1` 會拒絕打包。

`main`／手動執行（選 `main`）使用 `dev-<run_number>.<run_attempt>-<短 commit>` 作為版本標籤；推送 `v*` 標籤時沿用該標籤。即使標籤為 `v1.0.0`，本流程仍只發布 Pre-release。已公開的同名版本不會被重跑覆寫；上傳未完成的草稿可重跑補齊。

建置環境使用 GitHub 的 `ubuntu-latest`／`windows-latest` 和每次安裝的最新 **Rust stable**（不是 nightly）。Actions 使用目前最新穩定版的固定 commit；Dependabot 每日檢查 Actions 與 Cargo 依賴並提出更新 PR，通過驗證及合併後才進入發布。套件使用已提交的 `Cargo.lock` 與 `--locked`，不在發布時偷偷改變依賴；LUKSbox 仍固定在已驗證的版本。Linux 的 glibc 相容性下限會隨 `ubuntu-latest` 映像更新。

本機也可用 PowerShell 7 在對應平台執行 `./package.ps1 -Archive`，產生相同壓縮檔及校驗檔。GitHub Actions 僅發布套件，不會提交原始碼；本機操作仍遵循 AGENTS.md 的禁止 push 規則。

## 資料儲存

預設資料目錄為使用者目錄下的 `.misf-management/`，可用 `MISF_DATA_DIR` 改變。

- 主機與憑證存於 `hosts.db`（SQLite，檔頭 `user_version` 記錄結構版本，目前為 v4）。開啟較舊版本會先在同目錄備份 `hosts.v<N>.bak.db`，再於單一交易內自動升級；較新版本、非 MISF、損壞或結構遭改動的資料庫一律拒絕開啟並保留原檔。每個版本的升級各自在 `core/src/db_upgrade/` 一個檔案。
- 從 JSON 版本升級時，第一次讀取會把同目錄的 `hosts.json` 匯入新的 `hosts.db`，原 JSON 保持不變（之後的修改只寫入 SQLite）。
- 非敏感偏好存於未加密的 `ui-settings.json`（`version: 1`）：語言、外觀、主視窗大小與是否最大化、設定視窗大小，以及主機清單顯示 `host_list`（等級 `detail`、自訂 `fields`、密集度 `density`：`comfortable`／`compact`，以及「未分類」在分類間的位置 `ungrouped_at`，省略時排在最後）、批次作業排序 `batch_order`（`sidebar`／`pending`／`recent`／`name`）。不保存憑證，主機庫鎖定時也能讀寫。
- 主機清單的分類、順序與收合狀態存於主機庫（`host_groups` 資料表與 `hosts.group_id`）。
- 主機資料序列化內容上限 64 MiB。

## 介面文字

介面文字放在 [`app/locales/zh-tw.json`](../app/locales/zh-tw.json) 與 [`app/locales/en-us.json`](../app/locales/en-us.json)，以固定 key 與 `{name}` 等命名參數維護。語系檔編譯進執行檔，發行套件不需另外安裝語言檔；修改 JSON 後需重新編譯。新增語言時，在 `app/src/i18n.rs` 登記並提供相同 key 與參數。測試會檢查 key 與參數是否一致；缺少翻譯時回退繁體中文。遠端命令原始輸出不翻譯，核心／系統的詳細錯誤亦保留原文。

## SSH 與套件管理細節

- 使用 russh 自帶 SSH，不呼叫系統 ssh，也不讀取電腦的 SSH config、known_hosts、SSH agent 或系統私鑰。首次連線先核對並保存 SHA-256 主機指紋，確認前不提交帳密；指紋變更立即阻擋認證。
- 系統偵測讀取 `uname` 與 `/etc/os-release`。偵測結果只來自主機本身，編輯主機不能改寫；位址或埠變更時清除。手動指定的套件管理器必須確實存在於主機上，否則在執行任何套件指令前停止。
- 檢查先更新索引（`apt-get update`／`dnf makecache --refresh`／`yum makecache`／`pkg update`），停用動態下載進度。
- 升級：APT 執行 `apt-get -y upgrade`（`DEBIAN_FRONTEND=noninteractive`、`--force-confold`、`NEEDRESTART_MODE=l`），只升級既有套件；DNF／YUM 執行 `-y upgrade`（可能依相依關係安裝或移除套件，新設定檔另存為 `.rpmnew`）；pkg 執行 `pkg upgrade -y`。索引更新上限 5 分鐘、升級上限 30 分鐘。
- FreeBSD／DragonFly 的偵測命令以 `sh -c` 包裝，`pkg` 命令以 `env` 設定環境變數，csh 登入 shell 也能執行。
- sudo 密碼以 SSH channel 標準輸入傳送，不放在命令列。PTY 預設開啟（類似 `ssh -t`）。
- 批次作業最多 8 台並行、單次最多 256 台；核心拒絕混合套件管理器的批次，執行時偵測到的套件管理器若與紀錄不同，該主機失敗而不執行。

## LUKSbox 加密儲存

使用 LUKSbox v0.6.1（固定 commit `69448f708f1328d672deb595e359579d630cb0b0`）的 core／format／VFS 函式庫；FIDO2 使用有修改紀錄的本機相容性副本，將 P-256 等依賴更新至新版。不掛載磁碟、不需要 WinFsp 或 TPM 整合。加密為 AES-256-GCM-SIV；密碼採 Argon2id（256 MiB、3 次、4 lanes）。這不是 LUKS2 磁碟格式。版本與仍未修補的 RSA 公告見 [依賴更新與安全公告](dependency-updates.md)，整合評估見 [LUKSbox 整合評估](luksbox-evaluation.md)。

- 啟用時建立容器、將資料直接從記憶體寫入，再關閉並重新解鎖驗證；原 `hosts.db` 先保留，再詢問是否刪除。
- 啟用後只讀寫 `hosts.lbx`；`hosts.lbx.enabled` 記錄已選擇加密模式。容器內保存 SQLite 資料庫映像 `/hosts.db`，只在記憶體中開啟，不寫出明文資料庫檔；舊容器內的 `/hosts.json` 會在下一次儲存時於同一次中繼資料提交轉成 `/hosts.db`。
- 解鎖後若容器內資料庫是舊結構版本，先詢問；同意才寫回新版本，取消則保持鎖定、檔案不變。鎖定、錯誤密碼或容器遺失時不會退回讀取明文主機庫。
- 密碼與 Passkey 合計最多 8 個槽。槽名稱存在加密容器內的主機資料庫。撤銷：核心禁止撤銷最後一個槽及不存在的槽；撤銷立即寫回目前容器，不刪除驗證器端憑證、不關閉已解鎖工作階段、不使舊備份失效。
- 明文副本刪除只處理該檔案（刪除 `hosts.db` 時一併刪除 `hosts.db-journal`），必須已成功解鎖，且不經資源回收筒；這不是安全抹除。
- `.lbx.header-bak`／`.lbx.meta-bak` 是上游的加密復原鏡像。
- 尚未提供原地改密碼、主金鑰輪替、TPM 或自動閒置鎖定。

### Passkey

Windows 使用 MISF 自身的 WebAuthn 請求介面，LUKSbox 繼續負責 PRF／容器金鑰包裝。每次註冊或驗證使用新的隨機 challenge 與合法 UTF-8 client JSON，不將固定 32-byte 零值當成 JSON。依 `WebAuthNGetApiVersionNumber` 選擇相容的 options 版本，保持必要的使用者驗證；較新 API 明確啟用 PRF。`0x80090027` 會顯示為原生參數錯誤，並附 API／options 版本。註冊後要求兩次驗證，取得相同的 PRF 結果後才保存 `Fido2HmacSecret` 槽。

憑證 ID 與公開 salt 儲存在 LUKSbox 槽內；PRF 結果參與包裝／解開容器主金鑰，不保存驗證器私鑰或 PIN。沿用上游 `luksbox.local` RP ID 與跨平台 salt 慣例，保留上游工具相容性。

Linux 使用系統 **libfido2** 與實體 CTAP2 安全金鑰；執行需對應的 libfido2 runtime（Ubuntu 通常為 `libfido2-1`）與 hidraw 存取權限。

實作來源：[LUKSbox FIDO2 說明](https://luksbox.penthertz.com/docs/keyslots/fido2)、[Microsoft WebAuthn API](https://learn.microsoft.com/en-us/windows/win32/api/webauthn/)。實體測試程序見 [Passkey 實體測試](passkey-testing.md)。

## 操作日誌

GUI 與 JSON CLI 都在資料目錄的 `logs/actions-YYYY-MM-DD.jsonl` 追加操作紀錄，依 **UTC 日期**分檔，每行一個 JSON。背景操作分別記錄開始與成功／失敗／阻擋；正常結束也有紀錄。日誌不進入加密容器。

允許欄位只有時間、程序 PID、固定動作代碼、結果、相關主機的顯示名稱與金鑰槽號；不保存 IP、埠、帳號、密碼、私鑰、指紋、Passkey 憑證 ID、PIN、檔案路徑、搜尋字串、表單值、遠端輸出或原始錯誤。若顯示名稱包含該主機已知的位址／憑證片段，會遮蔽並移除控制字元。背景操作在寫入開始紀錄失敗時不執行。日誌為明文且沒有防竄改保證，目前不自動清理。

## 隔離測試目錄

GUI 與 JSON CLI 都支援 `--debug.dir`（也支援 `--debug.dir <目錄>` 或 `--debug.dir= <目錄>`）：

```powershell
.\misf.exe --debug.dir="C:\temp\misf-test"
cargo run --locked -p misf -- --debug.dir="C:\temp\misf-test"
.\run.ps1 --debug.dir="C:\temp\misf-test"
```

```sh
./misf --debug.dir=/tmp/misf-test
```

指定後，主機庫、`ui-settings.json` 與 `logs/` 全部使用該目錄，優先於 `MISF_DATA_DIR`；不讀取或遷移預設使用者主機庫，指定失敗直接報錯，禁止回退正式資料，也禁止指定預設正式資料目錄。標題與介面持續顯示 DEBUG。

此模式只允許連線至明確的 loopback IP（`127.0.0.1`／`::1` 等），阻擋外部 SSH 和所有真實 Passkey 註冊／驗證；加密備份也只能匯出至測試目錄內。

實體測試可明確加上 **`--debug.allow-passkey`**，僅在同時提供 `--debug.dir` 時接受。這會允許在測試主機庫使用真實驗證器，可能於安全金鑰或 Hello 建立憑證；請使用測試金鑰。介面持續顯示已啟用硬體操作的 DEBUG 提示，日誌記錄 `debug_passkey_enabled`。

## 測試

```powershell
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Rust 整合測試在 loopback 啟動測試 SSH 伺服器，驗證密碼／私鑰、指紋、PTY、sudo、系統偵測、APT 與 DNF 檢查／升級、套件管理器不符與混合批次阻擋，不接觸真實遠端主機。

LUKSbox 測試使用暫存資料，涵蓋明文保留、確認刪除與路徑限制、鎖定後拒絕操作、錯誤密碼、多 keyslot、重新啟動、加密備份、縮短資料、密文損壞及並行開啟阻擋。Passkey 核心流程由模擬驗證器測試，未代替實體 Windows Hello／USB 裝置驗證。

## 畫面擷取

開發用畫面擷取可使用 `cargo build -p misf --features capture`，以 `MISF_SCREENSHOT_TO` 指定 PNG 路徑，並以 `--debug.dir` 指定測試資料。`MISF_OPEN_EDITOR=1`／`MISF_OPEN_SETTINGS=1`／`MISF_OPEN_PASSKEY=1`／`MISF_OPEN_BATCH=1`／`MISF_OPEN_KEYS=1` 可預先開啟對應畫面；旗標只有明確設為 `1` 才啟用；`MISF_OPEN_SETTINGS=about`／`MISF_OPEN_SETTINGS=preferences` 直接開啟「關於」／「偏好設定」頁。debug 模式的 Passkey 按鈕停用，擷取不操作硬體。正式 release 預設不啟用擷取功能。

`.daybrake` 不是 daybreak 拼錯，是故意的。
