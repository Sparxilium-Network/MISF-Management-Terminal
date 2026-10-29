# Passkey 實體測試

使用空的測試資料目錄與測試用驗證器。以下流程不讀取預設主機庫；`--debug.allow-passkey` 明確允許註冊真實憑證，外部 SSH 與備份限制維持啟用。不需要重設金鑰、清除 PIN 或刪除現有 Passkey。

```powershell
.\misf.exe --debug.dir="C:\temp\misf-passkey-test" --debug.allow-passkey
```

1. 確認畫面頂端顯示「已允許真實 Passkey 操作」。在空主機庫啟用 LUKSbox，設定一組僅用於此測試、至少 12 字元的密碼。保留它作為備援解鎖方式。
2. 若有多支 USB 金鑰，先只插入本次測試那支。開啟「管理解鎖金鑰」→「新增 Passkey…」→選 **FIDO2 安全金鑰**（測試 Windows Hello 時才選 Hello）。
3. 在 Windows 原生視窗完成註冊及接續的兩次驗證。可能多次要求觸碰或 PIN；PIN 僅輸入 Windows 視窗，勿回傳給開發者。成功後記下顯示的槽號。
4. 按「鎖定」，以「使用 Passkey 解鎖…」及同一驗證器重新開啟主機庫。重新啟動測試 App 後再驗證一次。
5. 如需測試備援金鑰，以備援密碼解鎖後插入另一支金鑰，註冊到另一槽，再分別驗證解鎖。
6. 撤銷指定 Passkey 槽後，鎖定並確認該金鑰不能再解鎖目前容器，備援密碼與其他金鑰仍可用。槽撤銷不刪除驗證器端憑證，也不使舊備份失效。

若失敗，回報發生步驟（註冊／第一次驗證／第二次驗證／重新解鎖）、驗證器品牌／型號／韌體及完整錯誤碼。新錯誤會包含 `WebAuthn API` 與 `options v` 等非敏感版本資訊。不要提供 PIN、私鑰、PRF 結果、主機資料或容器。原有明文操作日誌仍不記錄這些敏感內容。

`0x80090027` 表示 Windows 拒絕呼叫參數。修正版使用合法 client JSON、每次新 challenge、相容 options 版本與 PRF 啟用方式。2026-10-29 使用者回報修正版實測成功；未記錄確切裝置型號／韌體及全部測試步驟，因此不推論所有 OpenSK、其他 USB 金鑰或 Windows Hello 均已驗證。

一手參考：

- [Microsoft：WEBAUTHN_CLIENT_DATA](https://learn.microsoft.com/en-us/windows/win32/api/webauthn/ns-webauthn-webauthn_client_data)
- [Microsoft：API 與結構版本、PRF 參數](https://github.com/microsoft/webauthn/blob/master/webauthn.h)
