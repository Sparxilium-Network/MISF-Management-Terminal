# 專案工作規則

- 專案名稱統一為 **MISF Management Terminal**；程式、套件與環境變數使用 `misf`／`MISF` 命名。
- 遠端儲存庫為 `https://github.com/Sparxilium-Network/MISF-Management-Terminal.git`。
- 完成任何一批原始碼、文件或設定修改後，**必須建立本機 Git commit**，不得只留下未提交的變更。提交前執行與變更相關的驗證；建置產物遵循 `.gitignore`。
- **禁止執行 `git push`**，也禁止以其他工具、API 或強制推送方式將本機變更送到遠端。只可建立本機提交。
- 提交時保留使用者既有工作，勿撤銷或覆蓋與任務無關的變更。
- 完成修改並提交後，執行 `.\package.ps1` 將可攜式版本輸出至 `dist\MISF`，確認 `dist\MISF\misf.exe` 為最新建置後再交付使用者。
