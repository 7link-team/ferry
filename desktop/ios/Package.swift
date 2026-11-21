// swift-tools-version:5.3
import PackageDescription

let package = Package(
    name: "client-navigation",
    platforms: [.iOS(.v13)],
    products: [.library(name: "client-navigation", type: .static, targets: ["ClientNavigation"])],
    dependencies: [.package(name: "Tauri", path: "../.tauri/tauri-api")],
    targets: [.target(name: "ClientNavigation", dependencies: [.byName(name: "Tauri")], path: "Sources")]
)
