_纯天然代码，爱来自 Codex。_

# WinWiFi

基于 Rust + winio WinUI3 的 Windows WiFi 实时监测工具。
应用会持续扫描并可视化本机无线网卡可见的 AP 信息，重点字段包含：

- BSSID
- SSID
- MODE（BSS 类型 + PHY 类型）
- CHAN
- RATE
- SIGNAL（百分比与 dBm）

## 技术栈

- Rust 1.93.1
- winio 0.10.0（`winui` + `plotters`）
- winio-winui3 0.3.8（由 winio-winui 后端链路提供）
- windows-sys 0.61.2（Native WiFi API）

## 运行

```bash
cargo run --release
```

## 静态检查

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
```

## 位置权限说明

Windows 对 WiFi BSSID 访问与位置权限联动。  
若位置权限未允许，`WlanGetAvailableNetworkList` / `WlanGetNetworkBssList` / `WlanScan` 可能返回 `ERROR_ACCESS_DENIED`。应用内会提示并提供一键打开系统设置入口。

## MIT
