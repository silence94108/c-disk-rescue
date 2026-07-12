# C盘救星（C-Disk Rescue）

> 免费、无捆绑、傻瓜式的 Windows **C 盘空间治理工具**——让完全不懂电脑的人也能安全地把 C 盘空间要回来。

![platform](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-0067c0)
![release](https://img.shields.io/github/v/release/silence94108/c-disk-rescue?label=下载)
![built with tauri](https://img.shields.io/badge/built%20with-Tauri%202-24c8db)

Windows 笔记本的 C 盘普遍偏小,软件却默认把缓存、聊天数据、更新包一股脑写进 C 盘。市面上的清理工具要么捆绑推广、清理项不透明,要么面向专业用户不敢给小白用。C盘救星想做的,是把"清理 + 迁移 + 排查"整条链路做到**每一步都看得懂、可撤销、用完即走**。

## 它能做什么

- **空间体检** — 扫描 C 盘,用分段容量条 + 目录树看清"空间到底去哪了"。
- **安全清理** — 内置白名单识别垃圾(系统/浏览器/开发缓存、回收站、缩略图、崩溃转储…),三级风险标注(放心删 / 有代价 / 谨慎),一键清理。
- **搬家瘦身** — 把微信、企业微信、QQ、钉钉、Volta 等大数据目录用 junction 搬到其他盘,软件无感照常用,随时可一键搬回。
- **大文件排查** — 按大小列出大文件,删除进回收站可反悔;系统关键文件只展示不可删。
- **更多** — 旧驱动包引导清理、废弃账户残留识别、**大文件夹自选搬家**、**已搬走目录(含非本工具搬的)一键搬回**、体检结果本地缓存(打开秒显示上次结果)。

## 下载安装

到 [**Releases**](https://github.com/silence94108/c-disk-rescue/releases) 下载,两种任选:

- **安装版** `c-disk-rescue_x.y.z_x64-setup.exe` — 双击安装,带开始菜单快捷方式和卸载项;缺 WebView2 时会自动引导安装。
- **便捷版** `c-disk-rescue_x.y.z_x64-portable.exe` — 免安装,拷到哪双击就用、用完删掉不留痕。需系统自带 WebView2(Win11 及绝大多数 Win10 都有)。

系统要求:**Windows 10 1809+ / Windows 11,x64**。首次运行若弹「Windows 已保护你的电脑」→ 点「更多信息」→「仍要运行」(安装包未做数字签名);若被 360 / 腾讯电脑管家等拦截,在其信任区放行即可。

## 安全与隐私(为什么敢用)

- **全程离线**,不上传任何数据,无遥测。
- **无广告、无捆绑、无后台常驻**,用完即走,不注册开机启动、不装驱动。
- **只清内置白名单目录**,未知目录只展示、不提供删除按钮。
- **删除进回收站、迁移可一键搬回**,所有操作步步可撤销。
- **删除遍历绝不穿透 junction/软链接**,防止经由联接误删其他盘的数据。

## 开发

需要 [Node.js](https://nodejs.org/) 18+、[Rust](https://www.rust-lang.org/) stable、Windows 环境。

```bash
npm install
npm run tauri dev      # 开发(热重载)
npm run tauri build    # 打包,产物在 src-tauri/target/release/bundle/
```

## 技术栈

**Tauri 2 + Vue 3 + TypeScript + Vite**;后端 Rust 承载文件系统重操作:

- 自研并行遍历扫描(crossbeam + `read_dir`,枚举自带元数据零额外系统调用)
- junction 事务化迁移(copy → 双校验 → 换名 → 建链,断电可回滚,可搬回)
- Restart Manager API 按文件锁判定占用(不依赖进程名)
- 回收站删除(`SHFileOperationW` + `FOF_ALLOWUNDO`)

## 目录结构

```
src/                  前端(pages 页面 / components 组件 / api 封装 / store 状态)
src-tauri/            Rust 后端
  src/scan.rs         扫描、大文件、自选候选、孤儿 profile、外部 junction
  src/cleaner.rs      清理引擎、占用检测、驱动包口径
  src/migrator.rs     junction 迁移 / 搬回 / 事务恢复
  src/rules.rs        知识库规则加载
  rules/rules.json    清理与迁移知识库(路径模式、白话解释、风险级)
docs/                 需求文档、设计规范、优化方案
```

## 发布(维护者)

打 tag 即由 GitHub Actions 自动在 Windows 上构建并发布**草稿 Release**,审核后手动 Publish:

```bash
# 先同步改 src-tauri/tauri.conf.json 与 package.json 的 version
git tag v0.1.0
git push origin v0.1.0
```

工作流见 [`.github/workflows/release.yml`](.github/workflows/release.yml)。
