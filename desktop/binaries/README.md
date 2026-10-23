Tauri 在打包时会把 `ferry-server` 作为 sidecar 一起装进安装包。

构建前把可执行文件按目标三元组命名放在这里，例如：

    cargo build --release -p ferry-server
    cp ../target/release/ferry-server ./ferry-server-aarch64-apple-darwin
