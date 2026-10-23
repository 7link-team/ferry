`source.png` 是占位标识（1024×1024），其余文件由它生成：

    npm run tauri -- icon icons/source.png --ios-color '' 2>/dev/null || npm run tauri -- icon icons/source.png

换正式 Logo 时替换 `source.png` 再重新执行，不要手改生成结果。

**本项目不构建移动应用**，所以生成后请删掉 `android/` 与 `ios/` 两个目录；
桌面与 Windows Store 需要的是本目录下的 `32x32.png`、`128x128.png`、
`icon.icns`、`icon.ico` 和 `Square*Logo.png`，`tauri.conf.json` 只引用这些。
