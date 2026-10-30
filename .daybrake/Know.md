# 已知安全問題

此處記錄 MISF Management Terminal 已確認、尚需在威脅模型中明確說明的安全限制。測試證據和完整重現範圍請看各項連結報告。

## A6：SQLite 合法資料頁的位元翻轉不會被偵測

- **狀態：** 已重現；SQLite 頁面完整性限制。
- **影響範圍：** 明文 `hosts.db` 若被有寫入權限的程式修改成另一個結構仍有效的值，MISF/SQLite 可能照常讀取。`quick_check` 驗證結構，不是資料認證碼。
- **加密模式界線：** 對鎖定中的 `hosts.lbx` 密文直接翻位元會被 LUKSbox 認證拒絕。測試中被接受的加密 A6 案例，是測試程式在 vault 解鎖後直接把修改過的 SQLite 映像交給內部 `write_image`；它沒有證明外部程式可透過 MISF 公開介面做到此事，也沒有實際篡改執行中 MISF 的記憶體。
- **所需能力：** 明文模式需要能寫入資料庫檔案。加密案例需要控制解鎖後的資料或寫入流程；單獨修改鎖定的加密檔案不足以重現。此測試不證明需要 root。
- **處理方式：** 不宣稱 SQLite 能偵測所有位元翻轉。若產品需求包含偵測合法值遭竄改，需另行設計有明確金鑰邊界的完整性驗證。
- **詳細測試與限制：** [A6 after-patch test report](20261031-sqlite-storage-security/after-patch-report.md#detailed-a6-valid-sqlite-page-bit-flip)
