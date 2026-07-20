# quota-float-Pro

`quota-float-Pro` 是基于 [change-42-yhmm/quota-float](https://github.com/change-42-yhmm/quota-float) 的二次开发版本。原项目提供 Codex 官方账号额度悬浮窗，本项目在保留原有官方账号额度读取能力的基础上，增加了 API/第三方兼容平台余额监督、主题系统和更完整的桌面端交互。

> 本项目只用于本机监督 Codex 额度/余额用量，提示额度大概什么时候会用完；不会保存 API Key、Cookie、账号数据、原始接口响应、提示词或聊天记录。

## 二改新增功能

- 保留官方 Codex 登录态额度读取：继续显示官方账号的 5 小时额度、本周额度、重置时间和重置机会。
- 增加 API 登录识别：当检测到 API/第三方兼容接口登录时，自动切换到余额展示，不再误提示 Codex 未登录。
- 兼容 CC Switch：可读取当前 Codex provider、真实 base URL、Usage Query、余额接口和本地 provider 配置。
- 兼容 Codex++：支持读取 `~/.codex-session-delete/settings.json` 的当前 relay 配置，以及激活 profile 中的 `authContents` / `configContents`。
- 支持第三方 USD 余额接口：自动探测常见余额路径，包括 `/v1/usage`、`/usage`、`/balance`、`/credits` 等。
- API 余额只显示 USD：不会用请求次数、daily usage 或非余额字段冒充余额。
- API 进度条按本机余额高水位计算：当前余额作为 100%，余额下降时进度同步下降，续费增加后重新回到 100%。
- 新增主题设置窗口：托盘菜单点击“主题”打开独立设置窗口，可切换主题、置顶、常态展开、轮播速度和进度条样式。
- 新增 7 套主题：极光、深色、青瓷、竹绿、孔雀绿、绿云、星河。
- 新增连续/分段进度条：分段进度条固定 5 段，并适配当前主题色。
- 优化悬浮窗交互：圆球/展开动画更顺滑，修复圆球变长方形、鼠标移入闪动、非默认主题灰色外圈等问题。

## 主题预览

| 极光 | 深色 |
| --- | --- |
| ![极光主题](docs/images/themes/theme-aurora.png) | ![深色主题](docs/images/themes/theme-dark.png) |

| 青瓷 | 竹绿 |
| --- | --- |
| ![青瓷主题](docs/images/themes/theme-qingci.png) | ![竹绿主题](docs/images/themes/theme-bamboo.png) |

| 孔雀绿 | 绿云 |
| --- | --- |
| ![孔雀绿主题](docs/images/themes/theme-peacock.png) | ![绿云主题](docs/images/themes/theme-lvyun.png) |

| 星河 |
| --- |
| ![星河主题](docs/images/themes/theme-xinghe.png) |

## 界面示例

| 官方账号额度 | API 余额 | 圆球模式 |
| --- | --- | --- |
| ![官方账号额度状态](docs/images/quota-states.png) | ![API 余额卡片](docs/images/quota-api-balance-card.png) | ![圆球模式](docs/images/quota-orb.png) |

## 支持的数据来源

- Codex Desktop 官方登录态：通过本机 Codex/Codex Desktop 登录状态读取官方额度。
- OpenAI API 或 OpenAI-compatible API：读取配置中的 `base_url`、`experimental_bearer_token`、`OPENAI_API_KEY` 或 auth 文件。
- CC Switch：读取当前 Codex provider 和 Usage Query 配置。
- Codex++：读取当前 relay/API 配置，切换 API 后下次刷新会重新识别。

如果第三方服务没有暴露可识别的 USD 余额接口，小组件会提示“已连接 API，但没有检测到可用的 USD 余额字段”，不会编造余额。

## 隐私边界

- 不保存 API Key、Codex token、Cookie、验证码或账号资料。
- 不保存原始额度响应、请求日志、提示词或聊天内容。
- 只保存小组件偏好设置和本机余额高水位基线，用于进度条计算。
- 余额接口请求只在本机配置的官方或第三方 API 地址上发起。
- 不包含遥测、统计、崩溃上报或第三方追踪。
- 不会兑换重置机会，也不会修改账号设置。

## 开发

环境要求：

- Node.js 20+
- Rust stable
- Tauri 2 桌面端依赖

```bash
npm install
npm test
npm run build
npm run tauri dev
```

Codex Desktop 更新后，可运行兼容性检查：

```bash
npm run check:codex
```

## 构建

```bash
npm run tauri build
```

Windows 下 Tauri 可能会下载 WiX 用于生成 MSI。如果 WiX 下载失败，release exe 仍可能生成在：

```text
src-tauri/target/release/quota-float.exe
```

## 自动更新

二改版自动更新已对接本仓库：

```text
https://github.com/Cokelce/quota-float-Pro/releases/latest/download/latest.json
```

发布新版本时，推送 `v*` tag 会触发 GitHub Actions 生成 Release 和 updater 所需的 `latest.json`。仓库 Settings -> Secrets and variables -> Actions 需要配置：

- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，如果生成私钥时没有设置密码可以留空不配

## 下载哪个包

- Windows 用户：下载 GitHub Releases 里的 `x64-setup.exe` Windows 安装包。
- Mac 用户：下载 GitHub Releases 里的 `universal.app.tar.gz` macOS Universal 包，适用于 Apple Silicon 和 Intel。
- 所有下载都在 [GitHub Releases](https://github.com/Cokelce/quota-float-Pro/releases/latest) 页面。

## Mac 详细使用方法

1. 打开 [GitHub Releases](https://github.com/Cokelce/quota-float-Pro/releases/latest)。
2. 下载 `Quota.Float.Pro_universal.app.tar.gz` 或名称里带 `universal.app.tar.gz` 的文件。
3. 双击解压，得到 `Quota Float Pro.app`。
4. 建议把 `Quota Float Pro.app` 拖到 `应用程序` 文件夹。
5. 第一次启动如果提示“无法验证开发者”或“已阻止打开”，不要直接双击打开；在 Finder 里右键 `Quota Float Pro.app`，选择“打开”，再点一次“打开”确认。
6. 如果仍然无法打开，可以在终端执行：

```bash
xattr -dr com.apple.quarantine "/Applications/Quota Float Pro.app"
```

然后再从 `应用程序` 里打开。

### Mac 上怎么显示

- 普通模式：会显示桌面悬浮小圆球，鼠标移上去展开卡片。
- 开启“状态栏进度条”后：桌面悬浮圆球会隐藏，进度显示在 macOS 顶部菜单栏。
- 鼠标移动到菜单栏进度图标上，会显示迷你额度卡片。
- 颜色会跟额度同步变化：健康、警告、危急。

### Mac 上读取额度的前提

- 官方账号：需要本机已经登录 Codex Desktop / Codex。
- API 或第三方兼容 API：需要本机已有 Codex、CC Switch 或 Codex++ 的配置。
- 如果第三方接口没有提供 USD 余额字段，小组件会提示未检测到余额接口，不会伪造余额。

### Mac 自动更新

应用启动后会自动检查 GitHub Releases 的更新。也可以右键菜单栏图标，点击 `Check for updates` 手动检查。

## 上游项目

- 上游项目：[change-42-yhmm/quota-float](https://github.com/change-42-yhmm/quota-float)
- 本项目为二次开发版本，感谢原作者提供的 Codex 额度悬浮窗基础实现。

## License

MIT
