# Passkey 實體裝置測試

- 2026-10-29：修正 `0x80090027`（Windows 拒絕呼叫參數）。修正內容：合法 client JSON、每次新 challenge、相容 options 版本與 PRF 啟用方式。
- 2026-10-29：使用者回報修正版實測成功。未記錄確切裝置型號／韌體及全部測試步驟，因此不推論所有 OpenSK、其他 USB 金鑰或 Windows Hello 均已驗證。
- 2026-10-30 依賴更新（FIDO2 p256 0.14 等）後未重新操作實體驗證器。
- Linux libfido2 路徑未在實體裝置驗證。
- 測試程序：`docs/passkey-testing.md`。
