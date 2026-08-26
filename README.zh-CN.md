<div align="center">

<img src="src-tauri/icons/icon.png" alt="桌面翻译" width="120" />

# 桌面翻译

**在桌面任意位置选中文字，就地完成翻译。**

[English](README.md) · 简体中文

</div>

桌面翻译是一款轻量的 macOS 菜单栏应用。你在邮件、PDF、终端或浏览器中选中文字后，
应用会在选区旁显示一个小按钮；只有点击按钮后，文本才会发送到你选择的翻译服务。

## 主要功能

- 划词翻译不会抢走当前应用的焦点，也支持菜单栏中的快速输入面板。
- 支持英文和简体中文界面，可从菜单栏或设置窗口即时切换。
- 支持 Google Cloud、百度翻译和微软翻译；微软可选择全球或中国区云环境。
- API 凭据通过原生安全输入框保存到系统钥匙串，不会进入 WebView 或设置文件。
- 原文和译文均可使用系统语音朗读。
- 自动建立本地个人词库，支持词性、发音、相关词、双向测试和记忆分数；划词时若能确定所在句子，会把该句保存为例句，中译英词条以英文为相关词锚点。
- 内置一册离线的英汉入门词书；无需下载、无需配置 API 即可开始学习。

![个人词库、记忆分数、词性和发音按钮](docs/screenshots/vocabulary-study-wordbook.png)

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/vocabulary-study-textbooks.png" alt="简体中文词书书架" /></td>
    <td width="50%"><img src="docs/screenshots/vocabulary-study-related.png" alt="相关词与来源标记" /></td>
  </tr>
</table>

翻译查询遵循固定顺序：**个人词库 → 当前词书 → 你选择的在线服务**。应用不会在后台
擅自切换在线服务。在线翻译失败时，会显示当前服务的错误，方便你检查网络、配额或凭据。

## 安装

### macOS

从 [最新 Release](https://github.com/Ldsystem/desktop-translator/releases/latest) 下载 `.dmg`，
将应用拖入“应用程序”文件夹。安装包为通用版本，同时支持 Apple 芯片和 Intel Mac，
并已包含界面、数据库运行库和离线入门词书，不需要安装 Node.js、Python 或 SQLite。

当前 Release 使用 ad-hoc 签名，首次启动前需要执行：

```sh
xattr -dr com.apple.quarantine "/Applications/Desktop Translator.app"
```

### Windows 10/11 x64

从同一 Releases 页面下载 `Desktop Translator_*_x64-setup.exe`（NSIS，当前用户安装）。
若本机没有 WebView2，安装程序会通过微软官方 bootstrapper 下载。不需要 Node.js、Rust、
Python 或管理员权限。构建**未经 Authenticode 签名**，SmartScreen 可能提示
“更多信息 → 仍要运行”。

本机 Windows 11 25H2 x64 已完成编译、CI 门禁和静默安装/启动/卸载；针对第二个应用的
划词叠加层、SAPI、凭据提示等交互夹具仍待手动执行。详见英文
[README.md](README.md) 与 [平台矩阵](docs/platform-test-matrix.md)。

## 初次设置

1. 在“系统设置 → 隐私与安全性 → 辅助功能”中允许桌面翻译，然后从菜单栏退出并重新打开。
2. 在设置中选择翻译服务：

| 服务 | 需要配置 | 说明 |
| --- | --- | --- |
| 百度翻译 | APP ID、密钥 | 推荐中国大陆用户优先尝试 |
| 微软翻译 | 订阅密钥、云环境，可选区域 | 中国区需要 Azure 中国账号 |
| Google Cloud | Cloud Translation API 密钥 | 可用性取决于本地网络 |

请在服务商后台限制 API 用途，并设置配额或预算。个人词库、练习结果和下载的词书都保存在
本机，不会同步到云端；句子式文本只会翻译，不会加入词库。

## 开发

需要 Node.js 20+、pnpm 10+ 和稳定版 Rust：

```sh
pnpm install
pnpm tauri dev
```

| 命令 | 用途 |
| --- | --- |
| `pnpm check` | TypeScript 检查、前端测试和构建 |
| `pnpm test:platform` | Rust 单元与集成测试 |
| `pnpm tauri build` | 生成自包含的 `.app` 和 `.dmg` |
| `pnpm tauri build --bundles nsis` | 生成 Windows x64 NSIS 安装包 |

详细的架构、平台支持和权限说明请参阅 [英文 README](README.md)。
