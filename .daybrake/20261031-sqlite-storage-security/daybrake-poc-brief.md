# DayBrake POC 任務書：MISF SQLite 儲存安全驗證

**給 GPT DayBrake。** 請在隔離環境中，獨立驗證 MISF Management Terminal 改用 SQLite 後的儲存安全。目標是用可重現的 POC 證明或推翻 [SQLite 儲存安全評估](sqlite-security.md) 中的每一項主張。不需要修改產品程式碼；若找到問題，請附上最小重現檔與修正建議。

## 規則

- 只能使用測試資料與 loopback 主機。不得使用真實憑證，也不得連線外部主機。
- 用 `--debug.dir=<空目錄>` 啟動，或設定 `MISF_DATA_DIR` 指向測試目錄。禁止指向 `%USERPROFILE%\.misf-management`。
- 只在本機操作：不要 `git push`，也不要對遠端做任何寫入。
- 回報時每一項都要標示「成立／不成立／無法判定」，並附上指令、輸入檔與觀察到的輸出。

## 背景

- 主機資料庫是 `hosts.db`（明文模式），或 LUKSbox 容器 `hosts.lbx` 內的 `/hosts.db` 映像（加密模式，只在記憶體中開啟）。
- 檔頭：`application_id = 0x4D495346`，`user_version = 1`。
- 資料表：`hosts`、`check_results`、`packages`，都是 STRICT 表。
- 開啟流程在 `core/src/sqlite.rs` 的 `prepare_with`：
  1. 強化連線（harden）；
  2. 執行 `quick_check`；
  3. 檢查 `application_id` 與版本；
  4. 比對 `sqlite_schema` 與參考結構；
  5. 依需要執行遷移。
- 建置與測試：

```bash
cargo test --workspace
```

```bash
cargo build --release -p misf
```

## 要驗證的項目

### A. 惡意資料庫檔（最高優先）

請製作以下 `hosts.db`。每一個都要放進測試資料目錄，或包進測試用 LUKSbox 容器，然後啟動 MISF 並執行「列出、儲存、刪除主機」：

1. 含 `AFTER INSERT ON hosts` trigger，會把 `password` 複製到另一張表。
2. 含 view、額外 index，或 `sqlite_stat1`／`sqlite_sequence` 表。
3. 欄位定義與原版只差大小寫或空白，例如 `password TEXT  NOT NULL`。
4. generated column 或 CHECK 中呼叫函式。
5. `application_id` 錯誤、`user_version` 為 0／2／99／-1。
6. 截斷、翻轉位元、頁面大小或 b-tree 損壞，可用 fuzz 工具。
7. 已開啟 WAL 的檔案，並附帶惡意 `hosts.db-wal`。

**預期結果**：
- 全部拒絕開啟，並顯示錯誤；第 6 項中，資料頁內仍為合法值的位元翻轉屬於明文模式已知限制（SQLite 頁面沒有認證碼），不算繞過；
- 原檔案不被修改（以雜湊比對）；
- 沒有任何 trigger 執行，資料目錄沒有新增檔案；
- 程式不當機。第 7 項請特別確認 WAL 內容是否在檢查前就已套用。

### B. 檔案殘留與加密模式

1. 加密模式下反覆執行新增、檢查、刪除、鎖定、解鎖。期間持續監看資料目錄（例如 Process Monitor 或輪詢檔案清單），確認從未出現明文 `*.db`、`*-journal`、`*-wal` 或暫存檔。
2. 在加密容器的密文中搜尋測試密碼的明文片段，結果應不存在。
3. 明文模式下刪除主機後，用 hex 檢視 `hosts.db`，確認舊密碼已被 `secure_delete` 覆寫。
4. 在明文模式寫入途中強制結束程序，檢查 `hosts.db-journal`。重新開啟後資料應該一致，而且「刪除明文副本」流程會一併刪除 journal。

### C. 升級、降級與匯入

1. 舊版 `hosts.json`（明文）與舊容器內的 `/hosts.json`，都應自動轉為 SQLite，且資料完全一致：
   - 明文模式下，原 JSON 保持不變；
   - 容器內的 JSON 在同一次提交中被移除。
2. 用測試分支加一個假的 v2 遷移，確認幾件事：
   - v1 檔案會先備份成 `hosts.v1.bak.db` 再升級；
   - 升級中途失敗會回滾；
   - v1 程式開啟 v2 檔案會被拒絕，且檔案不被修改。

### D. 執行環境

1. 用 `dumpbin /imports misf.exe` 或同等工具，確認執行檔沒有載入 `sqlite3.dll`。把一個假的 `sqlite3.dll` 放在 exe 旁邊，確認不會被載入。
2. 兩個 MISF 程序同時寫入同一資料目錄，應該互斥且不損毀資料。另以 `sqlite3` CLI 長時間鎖住檔案時，MISF 應在約 5 秒後報錯，而不是卡住。
3. 確認 Linux／macOS 上 `hosts.db` 與 `hosts.v*.bak.db` 的權限是 0600。

### E. 劫持情境（評估影響，不需要修補）

在明文模式下模擬同帳號惡意程式：把某台主機的 `address` 與 `fingerprint` 改為 loopback 上的假 SSH 伺服器，記錄 MISF 會送出哪些憑證。再於加密模式重做一次，確認改不了內容，或改了之後會解鎖失敗。這一項用來量化「明文模式不防同帳號攻擊者」這條已知限制。

## 交付物

1. 一份回報 Markdown，逐項列出 A1–E1 的結論與證據。
2. 每個成立的問題各附一份最小重現檔或腳本，放在 `poc/` 目錄，不要加入產品程式。
3. 一份修正建議清單，依嚴重度排序。
