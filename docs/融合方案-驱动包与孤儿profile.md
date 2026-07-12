# 融合方案:驱动包清理 & 孤儿 profile 检测

> 版本:v0.1(草案,待 review)
> 配套文档:[需求文档.md](./需求文档.md) · [设计规范.md](./设计规范.md)
> 来源:真机排查中发现的两类真实垃圾,沉淀为产品能力。归属里程碑 **M4 打磨**。

## 0. 背景与结论先行

在一台真实用户机器上排查 C 盘时,发现两类现有引擎覆盖不到的空间浪费:

1. **cleanmgr「设备驱动程序包」9.57 GB** —— Windows 更新驱动后堆积在 `DriverStore\FileRepository` 的旧版本驱动包。
2. **`C:\Users\` 下的乱码孤儿目录 ~230 MB** —— 第三方软件(腾讯电脑管家等)以异常用户名创建、只含 `AppData` 的废弃 profile 残骸(`өө` / `鐣呯晠` / `����`)。

**核心判断:这两点对应本项目架构里两种不同的机制,不能用同一种方式实现。**

| 融合点 | 本质 | 落点 | 改动量 |
|--------|------|------|--------|
| 驱动包清理 | **已知安全路径**,但删除需系统 API 判定 | 知识库 `rules.json` + `cleaner.rs` 特判 | 小 |
| 孤儿 profile | **启发式判断未知目录** | `scan.rs` 扫描引擎加一层检测 + 新契约 | 中 |

> 分界线依据:确定性路径走知识库哈希表精确命中(`scan.rs:240-247`);启发式判断必须写进 `scan_one_dir`(`scan.rs:164`)的运行时枚举逻辑——一个固定路径描述不了「有没有 NTUSER.DAT、是不是只有 AppData」这种特征。

---

## 1. 融合点一:驱动包清理

### 1.1 是什么

`C:\Windows\System32\DriverStore\FileRepository` 是 Windows 的驱动仓库。每次安装/更新驱动都会在此预存一份副本,**设计上只增不减**(保留回滚能力)。显卡(NVIDIA/AMD)驱动包单个数百 MB,更新十几次即累积到 GB 级。cleanmgr 的「设备驱动程序包」清理的正是**已被新版本取代的旧驱动包**,当前在用驱动不受影响。

### 1.2 关键约束:不能用「删目录内容」实现

⚠️ **这是最容易踩的坑。** 现有 `delete_contents`(`cleaner.rs:418`)是直接 `remove_file` 遍历删除。但 `FileRepository` 里**在用驱动和废弃旧驱动混在同一层目录**,无脑删会删掉正在用的驱动,导致设备失效。

正确做法:调用 Windows 的驱动管理接口,由**系统判定**哪些驱动包是孤儿(无设备在用):
- `pnputil /enum-drivers` 枚举 → 找出 published name 未被任何设备引用的包 → `pnputil /delete-driver oemXX.inf`;
- 或走 cleanmgr 的 `DRIVERS` handler(注册表 `VolumeCaches` + `SAGERUN`)。

### 1.3 落地方式:复用 `windows-old` 的「只展示不执行」模式

本项目已有先例。`windows-old` 规则(`rules.json:187`)因归属 TrustedInstaller、无法直接删,被特判为**只展示、不执行**,等待系统清理接口(`cleaner.rs:529-535`):

```rust
if rule.id == "windows-old" {
    total.skipped.push(SkippedRule {
        rule_id: rule.id,
        reason: "此项将在后续版本通过系统清理接口支持".into(),
    });
    continue;
}
```

驱动包与之**同类**,应复用此模式。

### 1.4 知识库规则(建议新增到 `rules.json`)

```json
{
  "id": "driver-packages",
  "pathPattern": "%WINDIR%\\System32\\DriverStore\\FileRepository",
  "displayName": "旧版本设备驱动备份",
  "explain": "更新驱动后系统留下的旧版本驱动备份。删除后无法一键回退到旧驱动,硬件用着正常就可以删。最近刚更新过显卡等驱动、还在观察是否稳定的,建议先留几天。",
  "risk": "cost",
  "needsAdmin": true,
  "relatedProcesses": [],
  "action": "clean"
}
```

字段选型理由:
- `risk: "cost"` —— 不是纯垃圾。删除的代价是**失去「回滚驱动程序」能力**(设备管理器里那个按钮会失效),与 `windows-old`「删了无法回退旧系统」同级。故**默认勾选但非放心删**,`explain` 里必须讲清这个代价(对齐设计规范 §3.3 四要素)。
- `needsAdmin: true` —— DriverStore 在 `%WINDIR%` 下,普通权限只读。对齐现有 `windows-temp` / `software-distribution` 的权限矩阵。
- **`measure_dir` 会算出整个 FileRepository 的大小**(含在用驱动),这个数字**偏大、会误导**。见下方待决问题①。

### 1.5 `cleaner.rs` 需要的特判(伪代码)

```rust
// do_clean 内,与 windows-old 并列
if rule.id == "driver-packages" {
    // 方案 A(首版):只展示不执行,同 windows-old
    total.skipped.push(SkippedRule {
        rule_id: rule.id,
        reason: "此项将在后续版本通过系统驱动清理接口支持".into(),
    });
    continue;
    // 方案 B(完整):调 pnputil 枚举孤儿驱动包并逐个 delete-driver
}
```

### 1.6 待决问题

- **① 大小口径失真**:`measure_dir` 统计整个目录,但真正可释放的只是「孤儿驱动包」子集。首版若走「只展示」,建议**不显示具体大小或显示「约 X GB(含在用驱动)」**,避免承诺无法兑现的释放量。完整方案需先 `pnputil /enum-drivers` 才能算准孤儿部分——这需要额外的系统调用设计。
- **② 首版做到哪一步**:方案 A(只展示,零风险,当天可交付)还是方案 B(真删,需封装 pnputil,风险与工作量都更高)。

---

## 2. 融合点二:孤儿 profile 检测(独家能力)

### 2.1 是什么

`C:\Users\` 下,某些第三方软件(已定位到腾讯电脑管家 QQPCMgr 的插件系统 radium/XPlugin)在异常上下文中会用**错误编码的用户名**创建 profile 目录,只生成 `AppData` 后废弃。真机样本:

| 目录(显示) | 真身 | 内容 | 大小 |
|-------------|------|------|------|
| `өө` | 西里尔字母残名 | 腾讯电脑管家插件运行时缓存 | 215.8 MB |
| `鐣呯晠` | "畅畅" UTF-8 被 GBK 误解码 | 企业微信/腾讯会议日志 | 5.3 MB |
| `����` | 字节已损坏 | 电脑管家 Temp、爱奇艺、Adobe cookie | 8.1 MB |

竞品普遍不检测这类目录。这是本项目可以做出差异化的点。

### 2.2 判定逻辑(采纳「三条件全满足」)

> 决策已确认:取最严口径,宁可漏报不误报,对齐产品红线「宁可少清 10GB,不能删错 1 个文件」(需求文档 §1.3)。

对 `C:\Users\` 的每个直接子目录,**三个条件同时成立**才判定为孤儿 profile:

```
1. 无 NTUSER.DAT          —— 不是真实登录用户的档案(真用户此文件必存在)
2. SID 未登记 Win32_UserProfile —— 系统不认这个 profile
3. 根目录几乎只含 AppData  —— 废弃 profile 的指纹(没有 Desktop/Documents/Pictures)
      ∧
⟹ 判定「孤儿账户残留」
```

**必须硬编码排除的白名单**(这些命中条件但绝不是孤儿):
`Default`、`Default User`、`Public`、`All Users`、当前登录用户目录,以及任何 `is_symlink()` 的目录(`Default User` / `All Users` 本就是 junction,red line:遇 reparse 不跟入)。

> 条件 2 的实现要点:`Win32_UserProfile` 需通过 WMI 或注册表 `HKLM\...\ProfileList` 读取已登记 SID 列表。若取数失败(如权限),应**降级为不判定**(返回空),绝不因取数失败误伤。

### 2.3 归属:F4 大文件/异常排查,而非 F2 清理

理由:
- F2 清理引擎只处理**知识库白名单里的固定路径**(`scan_cleanables` 遍历 `load_rules()`)。孤儿 profile 是运行时扫出的动态路径,不属于任何固定规则。
- F4 大文件页已有 `deletable` + `reason` 的「展示 + 白话解释」骨架(`scan.rs:453-463`、`BigFiles.vue:149-174`),孤儿目录可复用同一套 UI 语言(标黄提示 + 说明)。

### 2.4 前后端契约(新增)

在 `types.ts` 新增结构(与 `BigFileInfo` 并列):

```typescript
export interface OrphanProfile {
  /** 显示名(可能是乱码,前端原样呈现) */
  name: string;
  /** 绝对路径,如 C:\Users\өө */
  path: string;
  sizeBytes: number;
  fileCount: number;
  /** 命中的软件线索,如 ["腾讯电脑管家", "企业微信"](据内部 AppData 子目录推断) */
  hints: string[];
}
```

Rust 侧对应 `scan.rs`,serde `rename_all = "camelCase"`,由新命令 `get_orphan_profiles` 返回(注册进 `lib.rs:14` 的 handler)。`hints` 的价值:让用户看到「哦这是电脑管家留下的」,大幅降低删除时的恐惧(设计规范 §1「不害怕」)。

### 2.5 检测在哪做

`C:\Users\` 只有个位数子目录,检测成本极低。两种时机:
- **A.** 扫描时顺带:`do_scan` 扫到深度 1 的 `Users\*` 时判定,结果存进 `ScanResult`。零额外遍历(扫描本就枚举这些目录)。
- **B.** `get_orphan_profiles` 被调用时现查:更简单,与扫描解耦,但要重新枚举一次 `C:\Users`(成本可忽略)。

建议 **B**,理由:与扫描主流程解耦,不给 65 万文件的热路径加任何逻辑;`C:\Users` 重新枚举一次几毫秒。

---

## 3. 红线调和:关于「报告页直接勾选删除」

> 决策已确认:允许在报告页勾选删除孤儿 profile。
> 但这与需求文档 §1.3 红线「**未知目录只展示、不提供删除按钮**」直接冲突。团团不回避这个张力,在此给出一条**安全的删除路径**来调和,而非简单照做。

### 3.1 冲突点

原红线的用意:防止工具对「它不理解的目录」提供一键删除,避免误删用户数据。孤儿 profile **恰好是未知目录**——名字都是乱码,工具「不认识」它。

### 3.2 调和方案:从「未知」升级为「已识别的特定模式」

孤儿 profile 不是普通未知目录,它是**被三条件精确识别、且内容已探明(只有 AppData 缓存)的特定残骸**。删除权限的开放必须满足以下**全部**防护,才不破坏红线精神:

1. **白名单校验(接口层,最关键)**:删除命令 `delete_orphan_profile(path)` 必须校验该 path **在本次 `get_orphan_profiles` 返回的结果集里**——照搬 `delete_big_file` 的双防线(`scan.rs:569-578`):

   ```rust
   let known = orphans.iter().any(|o| o.path.to_lowercase() == path.to_lowercase());
   if !known {
       return Err("这个目录不在本次识别的残留列表里,拒绝删除".into());
   }
   ```
   这杜绝了「任意路径删除」——前端传什么路径都没用,不在识别集里一律拒绝。

2. **进回收站,不永久删**:复用 `recycle_delete`(`scan.rs:530`,`FOF_ALLOWUNDO`)。~230MB 远小于回收站上限,可反悔。

3. **二次确认 + 内容透明**:删除前必须让用户能「打开位置」看一眼;确认文案点明「这是软件留下的缓存残留,不含你的个人文件」+ 明确的「删除」「先不删」双按钮(设计规范 §5)。

4. **默认不勾选**:即便在报告页可勾选,**默认勾选状态为 false**——用户主动勾才删,不随「一键优化」无感执行。等价于 `caution` 级的谨慎态度。

### 3.3 建议:更新需求文档 §1.3 的措辞

现措辞「未知目录只展示、不提供删除按钮」应细化为:

> 未知目录只展示、不提供删除按钮;**但经知识库明确识别为特定残骸模式(如孤儿 profile)、且删除走回收站 + 接口层白名单校验 + 二次确认的目录,可提供默认不勾选的删除入口。**

这样红线依然守住「不删工具不理解的东西」,同时给「工具已充分理解的特定残骸」开了一条受控的删除路径。

---

## 4. 交付清单(待 review 后实施)

融合点一(驱动包):
- [ ] `rules.json` 新增 `driver-packages` 规则
- [ ] `cleaner.rs` `do_clean` / `scan_cleanables` 增加 `driver-packages` 特判(首版走「只展示」)
- [ ] 定夺待决问题 ①(大小口径)②(首版做到方案 A 还是 B)

融合点二(孤儿 profile):
- [ ] `scan.rs` 新增 `OrphanProfile` 结构 + `get_orphan_profiles` 命令 + `delete_orphan_profile` 命令(白名单校验)
- [ ] `lib.rs` 注册两个新命令
- [ ] `types.ts` 新增 `OrphanProfile` 类型;`api/index.ts` 封装
- [ ] `BigFiles.vue` 或报告页新增孤儿 profile 区块(标黄 + hints + 默认不勾选 + 二次确认)
- [ ] 需求文档 §1.3 措辞更新(见 3.3)
- [ ] 测试:中文/乱码用户名、`Win32_UserProfile` 取数失败降级、白名单目录不误报

## 5. 风险登记

| 风险 | 对策 |
|------|------|
| 驱动包「只展示大小」误导用户以为能释放这么多 | 首版不显示精确大小或标注「含在用驱动」;完整方案先 enum-drivers 算准孤儿部分 |
| 孤儿 profile 误判某些特殊系统目录 | 三条件全满足 + 硬编码白名单 + `Win32_UserProfile` 取数失败降级为不判定 |
| 开放删除权限被滥用为任意路径删除 | 接口层白名单校验(照搬 `delete_big_file` 双防线),前端传任意路径一律拒绝 |
| 乱码路径在 Rust/Shell API 边界出错 | 全程 `OsStr`/宽字符,不做 String 往返;中文/乱码路径列为一级测试用例(设计规范 §7) |
