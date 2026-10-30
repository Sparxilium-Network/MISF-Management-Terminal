# MISF 圖示素材

`icons/` 內的 11 個檔案由使用者提供的 `misf-icons.zip` 原樣匯入，均納入 Git 版本控制。保留原始檔名，不於建置時重新繪製或縮放。

| 檔案 | 用途 |
| --- | --- |
| `icons/misf-icon-1024.png` | 1024 × 1024、RGBA 透明背景原始圖檔 |
| `icons/misf-icon.svg` | 使用者提供的向量版本；與 PNG 的漸層、細節可能不同 |
| `icons/misf.ico` | Windows 執行檔資源，含 16、24、32、48、64、128、256 像素 |
| `icons/misf-{16,24,32,48,64,128,256,512}.png` | 各尺寸透明 PNG；程式視窗內嵌使用 256 像素版本 |
| `linux/org.sparxilium.misf.desktop` | Linux 應用程式選單項目，與視窗的 application ID 對應 |

Windows 使用 `app/build.rs` 與 `winresource` 將 ICO 編入 `misf.exe`，視窗圖示則由 `app/src/main.rs` 直接內嵌 PNG。執行時不需要另外讀取圖示檔。

`package.ps1` 在兩個平台的套件附上本資料夾。Linux 套件另附 `share/applications/org.sparxilium.misf.desktop` 與 `share/icons/hicolor/`，供使用者安裝選單圖示；單純執行 `./misf` 不會修改桌面設定。

Linux 使用者可在解壓後的套件目錄執行以下指令，安裝到自己的帳號。`Exec=misf` 需要 `~/.local/bin` 已在 `PATH` 中；系統的圖示或選單快取可能需要重新登入才更新。

```sh
install -Dm755 misf "$HOME/.local/bin/misf"
install -Dm644 share/applications/org.sparxilium.misf.desktop \
  "$HOME/.local/share/applications/org.sparxilium.misf.desktop"
for size in 16 24 32 48 64 128 256 512; do
  install -Dm644 "share/icons/hicolor/${size}x${size}/apps/misf.png" \
    "$HOME/.local/share/icons/hicolor/${size}x${size}/apps/misf.png"
done
install -Dm644 share/icons/hicolor/scalable/apps/misf.svg \
  "$HOME/.local/share/icons/hicolor/scalable/apps/misf.svg"
```
