# SQLite 儲存安全評估

本文件評估 MISF 改用 SQLite（`hosts.db`）後的運作安全性，重點是資料庫本身會不會被用來劫持 MISF 或竊取憑證。實作見 `core/src/sqlite.rs`、`core/src/store.rs`、`core/src/vault.rs`。外部驗證的任務說明見 [DayBrake POC 任務書](daybrake-poc-brief.md)。

## 結論

- **SQL 注入**：不可行。程式只有固定 SQL 與綁定參數，不會用使用者輸入組合 SQL。
- **惡意或被替換的資料庫檔**（匯入舊 DB、他人提供的檔案、被改過的 `hosts.db`）：
  - 已封鎖資料庫檔「自帶程式碼」的途徑。trigger、view、額外 index、ATTACH 與擴充套件都會被拒絕或停用，不會在 MISF 讀寫時執行。
  - 剩下的風險是 SQLite 解析畸形檔案時的記憶體安全漏洞，必須持續更新內建的 SQLite。
- **DLL 劫持**：不適用。SQLite 以 bundled 方式靜態連結進 `misf.exe`。檢查結果：執行檔不含 `sqlite3.dll` 字串，含 3.54.1 版本字串。
- **同一 Windows 帳號下的本機攻擊者**：明文模式下，能寫入資料目錄的程式就能改 `hosts.db`。這是最實際的「劫持」途徑，與舊版 JSON 相同，**不是** SQLite 新增的風險：
  - 把主機位址與指紋換成攻擊者的伺服器，下次連線時 SSH 密碼或 sudo 密碼就會送給對方；
  - 也可以直接讀出憑證。
  - **建議啟用 LUKSbox 加密模式。** 容器內容經 AES-256-GCM-SIV 驗證，竄改會解鎖失敗並拒絕開啟。
- **加密模式**：SQLite 資料庫只以記憶體映像存在，不會寫出明文 `.db`、journal 或暫存檔。

## 威脅與對策

| # | 威脅 | 對策 | 殘餘風險 |
|---|---|---|---|
| T1 | SQL 注入 | 全部語句寫死，值一律綁定參數（`params!`）；沒有動態表名或 `format!` 組 SQL。 | 無 |
| T2 | 惡意 trigger：例如每次寫入主機時把 `password` 複製到別的表或檔案 | 停用 trigger 與 view（`SQLITE_DBCONFIG_ENABLE_TRIGGER/VIEW=0`）；把 `sqlite_schema` 與同版本的參考結構逐列比對，多出任何物件就拒絕開啟。 | 無（有單元測試） |
| T3 | 結構中的函式呼叫：generated column、CHECK、DEFAULT 裡放函式 | `trusted_schema=OFF`、DEFENSIVE；結構 SQL 文字必須與參考完全相同。 | 無 |
| T4 | 用 `ATTACH` 或 `VACUUM INTO` 寫出任意檔案 | `SQLITE_LIMIT_ATTACHED=0`；不執行外來 SQL。 | 無 |
| T5 | 載入擴充 DLL | rusqlite 未啟用 `load_extension`；SQLite 預設關閉。 | 無 |
| T6 | 畸形 b-tree 或頁面，觸發 SQLite 解析漏洞 | `cell_size_check=ON`、`mmap_size=0`、開啟時執行 `quick_check`；bundled 3.54.1。 | SQLite 本身的 0-day；需隨上游更新（見 `docs/dependency-updates.md`） |
| T7 | 版本降級或升級攻擊：把較新或偽造版本號的檔案給舊版 MISF | `user_version` 大於支援版本就拒絕開啟，不寫入；`application_id` 必須是 MISF。 | 無 |
| T8 | 遷移中斷導致資料毀損 | 明文檔遷移前先複製 `hosts.v<N>.bak.db`，再於單一交易遷移，失敗即回滾；備份失敗則不遷移（有測試）。 | 備份檔是明文副本，需自行管理 |
| T9 | 同帳號本機程式竄改或讀取 `hosts.db` | 明文模式無法防護；Unix 上檔案權限為 0600，Windows 繼承使用者設定檔 ACL；加密模式有完整性驗證。 | **明文模式的主要風險**，建議改用加密 |
| T10 | 殘留明文：WAL、journal、暫存檔、已刪除的列 | 使用 rollback journal（不用 WAL，沒有長存的 `-wal`／`-shm`）；`secure_delete=ON`；`temp_store=MEMORY`；刪除明文副本時一併刪除 `-journal`；加密模式每次儲存都產生全新映像。 | SSD 損耗平均與檔案系統快照不在控制範圍 |
| T11 | 同時有多個程序寫入 | `hosts.lock` 跨程序互斥，加上 SQLite 鎖與 5 秒 busy_timeout；整份讀取後在單一交易寫回。 | 其他工具長時間鎖住檔案時，MISF 會報錯而不是等待 |
| T12 | 加密容器回滾：用舊備份覆蓋 `hosts.lbx` | LUKSbox 不防止回滾；舊備份可用舊密碼或已撤銷的金鑰解鎖。 | 已知限制，與改用 SQLite 無關 |
| T13 | 記憶體中的明文 | 映像緩衝以 `Zeroizing` 清除。 | SQLite 頁快取與 `Host` 字串釋放時不會清零；當機傾印或分頁檔可能含有憑證 |
| T14 | 透過 symlink 或 junction 改向資料檔 | 刪除明文副本時拒絕 symlink 與 reparse point；開啟資料庫沿用目錄。 | 需要對資料目錄有寫入權，等同 T9 |

## 版本槽與升級規則

- **識別**：`PRAGMA application_id = 0x4D495346`（"MISF"）。全新的空檔是 0/0，可以初始化；其他值一律拒絕。
- **版本**：`PRAGMA user_version`，目前為 1。
- **新增版本**：在 `MIGRATIONS` 尾端追加語句即可，已發佈的語句不可修改，因為開檔時會用這些語句重建參考結構來比對。
- **匯入或開啟舊 DB**：
  - 依 `user_version` 從該版本開始套用後續遷移；
  - 明文檔會先備份；
  - 加密映像在記憶體中升級，下次儲存時寫回。

## 尚待外部驗證

以下項目已有單元測試或人工檢查，但建議交由 DayBrake 獨立做 POC：

1. 精心製作的 `hosts.db`，能否在 MISF 開啟或寫入時執行程式碼、寫出檔案或外洩欄位。
2. 結構比對有沒有繞過方式，例如 `sqlite_schema` 的大小寫、空白、`sqlite_sequence`、`sqlite_stat1`。
3. 開啟 WAL 模式的惡意檔案時，`-wal` 內容會不會在 `journal_mode=DELETE` 生效前被套用。
4. Windows 上 `misf.exe` 的匯入表確實沒有 `sqlite3.dll`。
5. 加密模式下，各種操作過程中資料目錄有沒有出現任何明文 `.db`、`-journal` 或暫存檔。
