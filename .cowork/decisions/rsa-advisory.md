# RUSTSEC-2023-0071（rsa）處理方式

## 2026-11-08：移除 RSA 支援

- 使用者決定移除 RSA，以消除公告命中。`core/Cargo.toml` 的 russh 功能從 `["ring", "rsa"]` 改為 `["ring"]`；`rsa` 與 `pkcs1` 離開依賴樹，`cargo audit` 通過。
- 實測：OpenSSH 格式的 RSA 私鑰在無 `rsa` 功能時仍能被 `decode_secret_key` 解碼（ssh-key 只做編碼解析），但無法簽章；PEM（`BEGIN RSA PRIVATE KEY`）回傳 `UnsupportedKeyType`。因此 `model::decode_private_key` 以文字標籤與演算法兩道檢查明確拒絕，並給出中文訊息；新增主機、新增共用金鑰與連線前都經過它。
- 伺服器只提供 RSA 主機金鑰時，russh 回傳 `NoCommonAlgo { kind: Key }`，`ssh::handshake_error` 改寫成說明原因的訊息。
- 保留：Ed25519、ECDSA nistp256／384／521、後量子混合金鑰交換 `mlkem768x25519-sha256`。ML-DSA 簽章金鑰 russh 0.65 本來就不支援，與此決定無關。
- 已存在主機庫中的 RSA 金鑰不會自動刪除；連線時顯示 RSA 不支援訊息，由使用者換金鑰。

## 2026-10-30：先前決定（已取代）

- 命中 `rsa 0.10.0-rc.18`；公告沒有已修補版本。`russh 0.65.0` 精確要求 `=0.10.0-rc.18`。
- 當時保留 RSA，不加入 audit ignore，`cargo audit` 維持失敗。
- 重新啟用條件：上游 `rsa` 發布修補版且 russh 允許使用。
