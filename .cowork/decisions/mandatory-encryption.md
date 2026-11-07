# 強制加密與解鎖密碼規則

2026-11-07（commit `af2c75e`）

- 使用者要求全面移除未加密模式。沒有 `hosts.lbx`／`hosts.lbx.enabled` 時核心拒絕所有主機操作（錯誤：「尚未建立加密主機庫…」），GUI 啟動直接開啟建立視窗。
- 全新設定檔直接以空資料庫建立容器，不寫出明文 `hosts.db`。有舊版 `hosts.db`／`hosts.json` 時，只在 `vault_create` 讀取並轉入，之後詢問是否刪除。
- 新密碼規則（使用者指定）：至少 8 字元，數字／字母／符號至少兩種；`is_numeric` 為數字、`is_alphabetic` 為字母（含中文）、其他非空白為符號，空白只計長度。實作 `misf_core::vault::passphrase_problem`，GUI 與核心各檢查一次。
- 原規則為至少 12 字元（2026-10-28 LUKSbox 整合時定）。改為 8 字元＋兩種字元是使用者需求；Argon2id INTERACTIVE（256 MiB、t=3、p=4）不變。
- 既有金鑰槽不重新驗證規則，舊密碼照常解鎖。
- 測試用密碼改為含數字（如 `first recovery phrase 1`、`test vault 2026`）；session 測試需先 `vault_create` 才能存主機，稽核紀錄多兩筆。
