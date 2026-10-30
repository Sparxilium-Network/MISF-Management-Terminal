# MISF Management Terminal

[GitHub 儲存庫](https://github.com/Sparxilium-Network/MISF-Management-Terminal)

> [!WARNING]
> **本專案仍處於開發階段，請勿用於正式環境。**
> 相關安全保護機制尚未完善。

全 Rust 原生桌面客戶端，以 eframe／egui 繪製畫面，內建 Rust SSH 核心。畫面與核心編譯成同一個執行檔，不需要 Java、瀏覽器或管理伺服器。

預設將主機與憑證存於使用者目錄 `.misf-management/hosts.json`。可選擇啟用 **LUKSbox 加密主機庫**，之後日常資料改存同目錄的 `hosts.lbx`，每次啟動先解鎖。用 `MISF_DATA_DIR` 可指定資料資料夾。

首次啟動會自動複製舊版 `.msif-management/hosts.json` 至新資料目錄，保留原檔，且不覆寫已存在的新資料。

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

打包後直接執行 `dist/MISF/misf.exe`。不需要安裝 Rust 或 Java。

圖示素材放在 [`assets/icons/`](assets/icons/)，已納入版本控制。Windows 執行檔與程式視窗均內嵌圖示；Linux 套件另附選單項目與各尺寸圖示，安裝方式見 [`assets/README.md`](assets/README.md)。

Linux／macOS 可使用 `cargo run --locked -p misf`，須在對應平台建置；Linux 建置需要 OpenGL、X11／Wayland、fontconfig 與檔案對話框相關開發套件。此版本已在 Windows 建置與驗證。

## 自動建置與 Pre-release

[GitHub Actions](https://github.com/Sparxilium-Network/MISF-Management-Terminal/actions/workflows/build-prerelease.yml) 在 `main` 更新、推送 `v*` 標籤或手動執行時，自動測試並建置 **Linux x86_64** 與 **Windows x86_64**。兩個平台的格式、Clippy、測試與 release 建置全部通過後，才上傳套件並公開 **Pre-release**，固定不標記為 Latest。PR 只驗證與產生 Actions artifacts，不發布。

- Windows：`MISF-Management-Terminal-windows-x86_64.zip`，解壓後執行 `misf.exe`。
- Linux：`MISF-Management-Terminal-linux-x86_64.tar.gz`，解壓後執行 `./misf`。這是原生動態連結套件，需要圖形桌面及相容的 glibc、OpenGL、X11／Wayland、fontconfig 等系統函式庫；不是 AppImage，也不保證支援較舊發行版。
- 每個套件內附 README、架構／加密評估文件及第三方授權；另附 SHA-256 校驗檔。

`main`／手動執行（選 `main`）使用 `dev-<run_number>.<run_attempt>-<短 commit>` 作為版本標籤；推送 `v*` 標籤時沿用該標籤。即使標籤為 `v1.0.0`，本流程仍只發布 Pre-release。已公開的同名版本不會被重跑覆寫；上傳未完成的草稿可重跑補齊。GitHub 的 Pre-release 仍顯示在 Releases 頁面，只是標記為預發行版，不是正式發行。

建置環境使用 GitHub 的 `ubuntu-latest`／`windows-latest` 和每次安裝的最新 **Rust stable**（不是 nightly）。Actions 使用目前最新穩定版的固定 commit；Dependabot 每日檢查 Actions 與 Cargo 依賴並提出更新 PR，通過驗證及合併後才進入發布。套件使用已提交的 `Cargo.lock` 與 `--locked`，不在發布時偷偷改變依賴；LUKSbox 仍固定在已驗證的版本。Linux 的 glibc 相容性下限會隨 `ubuntu-latest` 映像更新。

本機也可用 PowerShell 7 在對應平台執行 `./package.ps1 -Archive`，產生相同壓縮檔及校驗檔。GitHub Actions 僅發布套件，不會提交原始碼；本機操作仍遵循 AGENTS.md 的禁止 push 規則。此工作流程需先由維護者推送至 GitHub 才會啟用，首次遠端執行結果請於 Actions 查看。

## 功能

點左上角 **設定** 開啟設定視窗，可選 **繁體中文（台灣）／English**，以及 **跟隨系統／淺色／深色**；切換立即生效，不需要重啟。非敏感偏好另存於資料目錄的 **未加密 `ui-settings.json`**，即使主機庫啟用 LUKSbox 也保持獨立，且不保存憑證。下次啟動自動沿用，主機庫鎖定時也能調整（先關閉解鎖提示即可開啟設定）。

介面文字放在 [`app/locales/zh-tw.json`](app/locales/zh-tw.json) 與 [`app/locales/en-us.json`](app/locales/en-us.json)，以固定 key 與 `{name}` 等命名參數維護。語系檔編譯進執行檔，發行套件不需另外安裝語言檔；修改 JSON 後需重新編譯。新增語言時，在 `app/src/i18n.rs` 登記並提供相同 key 與參數。測試會檢查 key 與參數是否一致；缺少翻譯時回退繁體中文。遠端命令原始輸出不翻譯，核心／系統的詳細錯誤亦保留原文。

- 保存多台 Linux／BSD 主機，支援新增、編輯、刪除與搜尋。
- SSH 帳密登入，或匯入 OpenSSH／PEM 私鑰；加密私鑰可填密語。
- 首次連線先核對並保存 SHA-256 主機指紋，確認前不提交帳密；指紋變更立即阻擋認證。
- 使用 russh 自帶 SSH，不呼叫系統 ssh，也不讀取電腦的 SSH config、known_hosts、SSH agent 或系統私鑰。
- 一鍵 APT 檢查：root 執行 apt-get update，一般使用者使用 sudo；更新索引成功後列出可更新套件、目前版本、候選版本與架構。檢查不安裝套件。執行輸出停用動態下載進度，保留來源與錯誤，支援文字選取及雙向捲動。
- **APT 升級**：確認對話框勾選了解風險後，先更新索引，再以 root／sudo 非互動執行 `apt-get -y upgrade`（`DEBIAN_FRONTEND=noninteractive`、保留既有設定檔 `--force-confold`、needrestart 僅列出 `NEEDRESTART_MODE=l`），上限 30 分鐘。只升級既有套件，不安裝新相依或移除套件；被保留的套件仍列為待更新。結果分別列出「已升級套件」（原版本／新版本）與剩餘待更新套件。升級會直接修改遠端系統，無法自動回復；新核心需自行重開機。
- **批次作業** 分頁：勾選多台已確認指紋的 Linux 主機，同時執行 APT 檢查或升級（最多 8 台並行，單次最多 256 台），各主機結果分別保存；單台失敗不影響其他主機。表格列出每台的上次操作、狀態、待更新與已升級數量，點主機名稱或「上一台／下一台」切換下方套件清單。BSD 與未確認指紋的主機無法勾選。
- 會連線或使用 sudo 的按鈕（測試連線、主機指紋、APT 檢查／升級、批次作業）及刪除主機，滑鼠停留約 1.5 秒會顯示將執行的實際命令、作用與風險。
- 可開關 PTY（預設開啟，類似 ssh -t）。sudo 密碼以 SSH channel 標準輸入傳送，不放在命令列。
- 失敗不顯示為零筆更新；保留最近一次檢查或升級結果與遮蔽憑證後的輸出。

BSD 支援 SSH 連線資料與連線測試，目前不提供 BSD 套件檢查。

## 可選的 LUKSbox 加密儲存

1. 按資料區底部的「啟用 LUKSbox 加密…」，設定至少 12 字元的解鎖密碼。
2. 程式建立容器、將資料直接從記憶體寫入，再關閉並重新解鎖驗證。原 `hosts.json` 先保留；驗證後顯示完整路徑，詢問是否刪除此明文副本。
3. 啟用後，新讀寫只使用 `hosts.lbx`；`hosts.lbx.enabled` 記錄已選擇加密模式。鎖定、錯誤密碼或容器遺失時不會退回讀取明文 JSON。按「鎖定」後，必須再次解鎖才能操作主機。
4. 透過「管理解鎖金鑰」新增備援密碼或 **Passkey**。密碼與 Passkey 合計最多 8 個槽，可分別解鎖同一容器。
5. 按「匯出加密備份…」建立新的 `.lbx` 備份檔。請保留可取回的備援密碼，並實際驗證備份可解鎖；遺失所有密碼將無法復原。

使用 LUKSbox v0.6.1（固定 commit `69448f708f1328d672deb595e359579d630cb0b0`）的 core／format／VFS 函式庫；FIDO2 使用有修改紀錄的本機相容性副本，將 P-256 等依賴更新至新版。不掛載磁碟、不需要 WinFsp 或 TPM 整合。加密為 AES-256-GCM-SIV；密碼採 Argon2id（256 MiB、3 次、4 lanes）。這不是 LUKS2 磁碟格式。版本與仍未修補的 RSA 公告見 [依賴更新與安全公告](docs/dependency-updates.md)。

尚未提供原地改密碼、主金鑰輪替、TPM 或自動閒置鎖定。備份或舊副本不會隨目前容器的變更自動失效。加密只保護未解鎖的檔案，無法保護已遭入侵的電腦。

### Passkey 與指定金鑰撤銷

先以密碼解鎖主機庫，再開啟「管理解鎖金鑰」→「新增 Passkey…」。Windows 可選 **Windows Hello** 或 **FIDO2 安全金鑰**，由原生 WebAuthn 視窗處理 PIN／指紋／臉部驗證。裝置必須支援 **PRF／hmac-secret**；依上游要求，Hello 需要 Windows 11 22H2 以上及相容設定，實際能力以註冊與驗證結果為準。註冊後會要求兩次驗證，取得相同的 PRF 結果後才保存 `Fido2HmacSecret` 槽。取消或驗證失敗不新增槽，但驗證器端可能已建立憑證。

Windows 使用 MISF 自身的 WebAuthn 請求介面，LUKSbox 繼續負責 PRF／容器金鑰包裝。每次註冊或驗證使用新的隨機 challenge 與合法 UTF-8 client JSON，不將固定 32-byte 零值當成 JSON。依 `WebAuthNGetApiVersionNumber` 選擇相容的 options 版本，保持必要的使用者驗證；較新 API 明確啟用 PRF。既有 RP ID、salt 前綴與容器格式維持相容。`0x80090027` 會顯示為原生參數錯誤，並附 API／options 版本，不再籠統標示為取消。

下次啟動可在解鎖提示選「使用 Passkey 解鎖…」。憑證 ID 與公開 salt 儲存在 LUKSbox 槽內；PRF 結果參與包裝／解開容器主金鑰，不保存驗證器私鑰或 PIN。沿用上游 `luksbox.local` RP ID 與跨平台 salt 慣例，保留上游工具相容性。

Linux 使用系統 **libfido2** 與實體 CTAP2 安全金鑰，MISF 要求提供該金鑰的 PIN，僅供當次操作，不保存。Linux 建置另需 `clang libclang-dev libfido2-dev`；執行套件需對應的 libfido2 runtime 與 hidraw 存取權限（Ubuntu 通常為 `libfido2-1`；可能需要發行版的裝置規則）。不把 Windows Hello 的憑證宣稱為跨裝置或雲端同步 Passkey。

「管理解鎖金鑰」按槽號列出密碼／Passkey，每個槽有「撤銷」。確認已另行驗證備援金鑰後才能提交，核心也禁止撤銷最後一個槽及不存在的槽。撤銷立即寫回目前容器；相同密碼若也登錄於其他槽，仍可經其他槽解鎖。此操作不刪除 Windows Hello／安全金鑰端的憑證，不關閉已解鎖工作階段，也不使舊備份失效。槽變更後應重新建立備份，並記住各槽對應的金鑰。

實作來源：[LUKSbox FIDO2 說明](https://luksbox.penthertz.com/docs/keyslots/fido2)、[Microsoft WebAuthn API](https://learn.microsoft.com/en-us/windows/win32/api/webauthn/)。自動測試使用模擬驗證器驗證真實容器的新增／解鎖／撤銷流程，未代替實體 Windows Hello／USB 裝置驗證。

## 未加密操作日誌

GUI 與 JSON CLI 都在資料目錄的 `logs/actions-YYYY-MM-DD.jsonl` 追加操作紀錄，依 **UTC 日期**分檔，每行一個 JSON。記錄開關設定、切換語言／外觀、主機選擇／搜尋／表單操作、匯入私鑰、確認／取消、SSH 探測／連線、APT 檢查／升級（批次作業每台主機各一筆）、升級確認／取消、分頁切換與批次勾選、主機保存／刪除，以及加密主機庫／金鑰／備份操作。背景操作分別記錄開始與成功／失敗／阻擋；正常結束也有紀錄。日誌不進入加密容器。

允許欄位只有時間、程序 PID、固定動作代碼、結果、相關主機的**顯示名稱**與金鑰槽號；不保存 IP、埠、帳號、密碼、私鑰、指紋、Passkey 憑證 ID、PIN、檔案路徑、搜尋字串、表單值、遠端輸出或原始錯誤。若顯示名稱包含該主機已知的位址／憑證片段，會遮蔽並移除控制字元；請勿在顯示名稱輸入其他敏感資料。表單變更只記錄動作，不記錄輸入內容。日誌寫入失敗會顯示錯誤；背景操作在寫入開始紀錄失敗時不執行。若操作已完成但結果紀錄寫入失敗，會明確提示先確認狀態再重試。

日誌為明文且沒有防竄改保證，目前不自動清理，需自行管理大小與保存期限。

## 隔離測試目錄

GUI 與 JSON CLI 都支援以下參數（也支援 `--debug.dir <目錄>` 或 `--debug.dir= <目錄>`）：

```powershell
.\misf.exe --debug.dir="C:\temp\misf-test"
# 開發環境
cargo run --locked -p misf -- --debug.dir="C:\temp\misf-test"
.\run.ps1 --debug.dir="C:\temp\misf-test"
```

```sh
./misf --debug.dir=/tmp/misf-test
```

指定後，主機 JSON／LUKSbox、`ui-settings.json` 與 `logs/` 全部使用該目錄，優先於 `MISF_DATA_DIR`；不讀取或遷移預設使用者主機庫，指定失敗直接報錯，禁止回退正式資料。請使用空目錄或測試資料。標題與介面持續顯示 DEBUG；禁止指定預設正式資料目錄。

此模式只允許連線至明確的 loopback IP（`127.0.0.1`／`::1` 等），阻擋外部 SSH 和所有真實 Passkey 註冊／驗證；加密備份也只能匯出至測試目錄內。可測試一般 UI、設定、明文／密碼加密主機庫與金鑰槽撤銷；Passkey 核心流程由模擬驗證器測試，不自動操作使用者的 Hello 或安全金鑰。

實體測試可明確加上 **`--debug.allow-passkey`**，僅在同時提供 `--debug.dir` 時接受。這會允許在測試主機庫使用真實驗證器，可能於安全金鑰或 Hello 建立憑證；請使用測試金鑰。介面持續顯示已啟用硬體操作的 DEBUG 提示，日誌記錄 `debug_passkey_enabled`。外部 SSH／備份位置限制仍生效；不會降級使用者驗證或關閉加密。測試程序見 [Passkey 實體測試](docs/passkey-testing.md)。

每次啟動及密碼／Passkey 解鎖成功後，會檢查目前資料目錄的 `hosts.json`。若已啟用加密且仍有明文副本，解鎖後會顯示欲刪除檔案的完整路徑，提供「保留檔案」與「確認刪除此檔案」。保留後，下次啟動或解鎖會再次詢問，也可從儲存區重新開啟確認。刪除只處理此檔案，必須已成功解鎖，且不經資源回收筒；這不是安全抹除。`ui-settings.json` 與 `logs/` 依原設計保持未加密且不會刪除。舊 `.msif-management` 目錄、外部備份與磁碟殘留需自行處理。啟用後請勿再用不支援 LUKSbox 的舊版操作同一資料目錄，以免重新更新明文副本。看到 `.lbx.header-bak`／`.lbx.meta-bak` 時，這些是上游的加密復原鏡像，請與容器妥善保存。主機資料序列化內容上限 64 MiB。

可用 [LUKSbox 原生工具](https://github.com/PentHertz/LUKSbox) 直接開啟備份；其容器內保存 `/hosts.json`。若要讓 MISF 使用備份，請先關閉程式，以備份建立獨立資料目錄並命名為 `hosts.lbx`，再透過 `MISF_DATA_DIR` 指向該目錄；避免覆寫目前主資料。詳細評估見 [LUKSbox 整合評估](docs/luksbox-evaluation.md)。

## 使用

新增主機後，選擇密碼或匯入私鑰。一般帳號可填 sudo 密碼；留空時沿用 SSH 密碼，Key 登入則需另填或設定免密 sudo。先核對主機指紋，再按「檢查 APT 更新」；確認清單後可按「APT 升級…」。多台主機改用「批次作業」分頁。APT 索引更新上限 5 分鐘、升級上限 30 分鐘，逾時會報錯；關閉程式會關閉本機連線，不能保證中止遠端已開始的 apt-get。建議先在單台測試主機升級，並避免同一叢集所有節點同時升級。

## 測試

```powershell
cargo test --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Rust 整合測試在 loopback 啟動測試 SSH 伺服器，驗證密碼／私鑰、指紋、PTY、sudo、APT 檢查／升級與批次作業，不接觸真實遠端主機。

LUKSbox 測試使用暫存資料，涵蓋明文保留、確認刪除與路徑限制、鎖定後拒絕操作、錯誤密碼、多 keyslot、重新啟動、加密備份、縮短資料、密文損壞及並行開啟阻擋。

## 結構

- `app/`：eframe／egui 原生 UI；背景執行緒執行網路及磁碟作業。
- `core/`：主機資料、SSH 及 APT 核心；亦提供 stdin/stdout JSON CLI。
- CLI 的加密工作階段只存在同一程序內；可透過 stdin 傳入 JSON 請求陣列，依序執行解鎖、操作與鎖定。不要把解鎖密碼放進命令列參數。
- `docs/architecture.md`：目前實作架構。

開發用畫面擷取可使用 `cargo build -p misf --features capture`，以 `MISF_SCREENSHOT_TO` 指定 PNG 路徑，並以 `--debug.dir` 指定測試資料。`MISF_OPEN_EDITOR=1`／`MISF_OPEN_SETTINGS=1`／`MISF_OPEN_PASSKEY=1`／`MISF_OPEN_BATCH=1` 可預先開啟對應畫面；旗標只有明確設為 `1` 才啟用。debug 模式的 Passkey 按鈕停用，擷取不操作硬體。正式 release 預設不啟用擷取功能。
