# Gongfu-Studio

功夫世界（软星 vEngine 引擎）游戏资源的逆向工程与 Mod 工具集。

| 目录 | 说明 |
|------|------|
| `cpk/` | CPK 资源包格式（`RST\x1A`）读写库，Cpack2009 官方打包器（CPKCore 2.0）的 Rust 移植 |
| `cpktool/` | 命令行工具：查看 / 列表 / 解包 / 打包 / 添加 / 删除 / 校验 |
| `gui/` | Gongfu Studio（见下节），从 gongfu-studio 迁入的 Tauri 2 + Vue 3 GUI；前端在 `gui/ui`，Rust 端在 `gui/src` |
| `cdp/` | CDP 渲染数据提取工具集（见 `cdp/README.md`）：驱动运行中的 Tauri 应用、读 viewer 内部蒙皮/锚定/穿模数据、抓 WebGL 帧 |

原 C++ 参考源码保留在 `G:\workspace\tools\Cpack2009`（MFC/x86，VS2026 可编译）；
游戏端 CPK 读取器在 `G:\pojie\ai\vEngine\vfs\CPKFast.cpp`（file system 1.0）。

---

## cpktool（CPK 打包 / 解包）

```text
cpktool info    <cpk>                          # 头部与统计
cpktool list    <cpk> [模式]... [--flat] [-l]  # 树形/平铺列表，-l 长格式
cpktool unpack  <cpk> [-o 目录] [模式]...      # 解包（目录模式自动含子树）
cpktool pack    <cpk> <路径>... [--no-compress]# 新建或追加打包（目录取内容进根）
cpktool add     <cpk> <路径>... [--no-compress]# 同 pack，要求包已存在
cpktool remove  <cpk> <模式>...                # 删除（数据变碎片，可复用）
cpktool verify  <cpk> [--quiet]                # 全量解压校验 MD5 + 路径 CRC 自检
```

通配符大小写不敏感，`*` 可跨目录；模式含 `\` 时按完整路径匹配，否则也匹配文件名。

```bash
cargo build --release -p cpk -p cpktool

# 解包整个包
cpktool unpack system.cpk -o out

# 只解包某个目录（含子树）与通配符
cpktool unpack system.cpk -o out "system\\AvatarGirl" "*.fskin"

# 修改后重新打包（out 目录内容进包根，与原包结构一致）
cpktool pack new_system.cpk out

# 向已有包覆盖添加（同名条目自动替换，旧数据变碎片）
cpktool add system.cpk 修改过的文件.dds
```

本机构建需保留 `.cargo/config.toml` 的 MSVC 链接器配置，其他机器可删。

### CPK 文件格式（逆向结论）

来源：`Cpack2009/CPKCore`（打包器 2.0）+ `vEngine/vfs/CPKFast`（游戏读取端 1.0）+
对 `system.cpk`（1.27GB / 20559 条目）、`g01_a.cpk` 的实测。

布局：

```
偏移 0      : 文件头 124 字节
data_start  : 数据区（建包时 = 124）
            :   若干条记录 [文件数据][简单名][00 00][附加信息 56B]
table_start : 表区（28B × max_table_num，位于包尾）
package_size: 包总大小 = table_start + 28 × max_table_num
```

文件头（124B = 31 个小端 DWORD）：

| 偏移 | 字段 | 说明 |
|-----|------|------|
| 0x00 | dwLable | 0x1A545352（"RST\x1A"） |
| 0x04 | dwVersion | 1（游戏端严格校验） |
| 0x08 | dwTableStart | 表区绝对偏移 |
| 0x0C | dwDataStart | 数据区起点（建包后不再更新） |
| 0x10 | dwFileNum | 有效文件 + 目录数 |
| 0x14 | dwIsFormatted | 未使用 |
| 0x18 | dwSizeOfHeader | 124 |
| 0x1C | dwValidTableNum | 有效表项数（含碎片） |
| 0x20 | dwMaxTableNum | 表容量（初始 5000，步进 5000） |
| 0x24 | dwFragmentNum | 碎片数 = valid − file |
| 0x28 | dwPackageSize | 包总大小 |
| 0x2C | dwReserved[20] | 保留 |

表项（28B，整表按 dwCRC 升序）：
`dwCRC / dwFlag / dwFatherCRC / dwStartPos / dwPackedSize / dwOriginSize / dwExtraInfoSize`。
dwFlag：`0x1` 有效、`0x2` 目录、`0x4` 大文件（未压缩）、`0x10` 已删除（碎片）、
`0x8000` CRC 重复；**高 16 位 = 过滤器编号**（1=存储，2=LZO1X-1，4=LZSS）。
删除仅置 0x10，数据保留为碎片供复用。

记录附加区：`dwExtraInfoSize = 简单名长度 + 2 + 56`，56B = 5 个 FILETIME(40B) +
MD5(16B)。MD5 对**未压缩原始数据**计算；原工具的时间戳是未初始化栈数据（调试版
为 0xCC 填充），游戏端不读取，本实现写真实时间。

CRC（文件身份/表键）：非反射 CRC32，多项式 0x04C11DB7、MSB-first、初值 = 前 ≤4
字节大端装载取反、逐字节 `(r<<8|b)^table[r>>24]`、最终取反；空串（根目录）= 0。
输入 = **全路径小写（仅 ASCII 折叠）反斜杠分隔的 GBK 字节**；简单名落盘保留原大小写。

压缩决策（打包时）：源文件 ≥ 50MB 或压缩后 > 95% 原大 → 直通存储（过滤器 1 +
0x4）；其余 LZO1X-1（minilzo 1.04 兼容）。无加密；LZSS（过滤器 4）核心库从不默认使用。

### 与原工具的已知差异

1. 空文件以存储方式打包（原工具因浮点 NaN 比较缺陷会对空文件写入 LZO 头，
   导致其自身无法解出）。
2. LZSS 压缩修正了原版结尾刷写位置错误（原版输出会被部分覆盖损坏）；
   解压方向与原版逐字节兼容。
3. 打包目录时取目录内容进包根（对齐「解包→修改→重打包」往返），原工具 GUI
   拖拽语义是把目录本身作为子目录。
4. 压缩输出与 minilzo 字节不同（同为合法 LZO1X 流，游戏可正常解压）。

限制：偏移全 32 位，单包上限 4GB；未实现碎片整理（原版也是空壳 TODO）；
MD5 校验跳过空文件（原工具对空内容记录的是打包机磁盘路径的 MD5，无意义）。

### 验证记录（2026-10-19）

- `system.cpk`（1.27GB，20559 条目）：全量校验 8s，20115 文件 MD5 全对，
  20559/20559 路径 CRC 一致。
- `g01_a.cpk`：6039 文件 MD5 全对，6044/6044 CRC 一致。
- 解包 `system\Boot\*`（278 文件，含 .fskin）→ 重打包 → 再解包：与源文件逐字节一致。
- 真实包副本上添加/覆盖/删除：碎片复用与计数规则与原版一致，全量校验通过。

---

## Gongfu Studio（模型预览 / 编辑器 / 动作预览）

`.fskin`（SKIN v1.0）游戏模型文件的预览与编辑器，含 `.fskel`（SKEL v1.0）骨架动画预览与导出、
`.fak`（AKK）怪物顶点动画模型预览，Tauri 2 + Vue 3 + Naive UI + three.js。

### 功能

- **角色装扮（换装变体重设计）**：自动发现角色资源根（含 `AvatarGirl`/`AvatarBoy`
  的目录，磁盘或 CPK 包内均可），基础模型男女一键切换；`system` 下的
  `Hat`/`Armonr`/`Lower`/`Glove`/`Boot` 为装备槽目录（帽子 `.fsm` 挂到头部骨骼
  随动作刚性跟随，其余为 `.fskin` 蒙皮件与基础模型共用骨架），`shizhuang` 为
  时装（单文件全身含头，脸型贴图从基础模型目录跨目录解析，穿上后基础头/身体
  部件自动隐藏、发型保留，身体装备槽置灰「被时装覆盖」）。装备文件名 `fs`*
  前缀 = 女款、`ms`* = 男款、其余（如 `FM01.FSM`）通用；前缀后数字为款式号，
  「套装」行按号一键整套切换 / ‹ › 翻页浏览（默认 = 全卸显示基础模型）；每个
  装备槽支持开关 + ‹ › 逐件快切 + 下拉直选；换性别时装备/时装按槽位+款式号
  自动迁移到对应性别款。检查器下方仍保留基础目录部件组（发型/款式变体），
  被替换的基础部件卡置灰并标注来源。数据扫描见 `gui/src/dress.rs`
  （`scan_character`，无材质的 FK55 类特效网格剔除）。
- **多文件装配**：扫描一个模型目录（如 `G:\pojie\ai\AvatarGirl`），根目录的基础部件
  （头/发/上衣/下装/靴子）自动装配成完整角色；`YiFu`、`MHair` 中的换装/发型变体按
  几何签名自动归组，下拉切换即可换装，开关控制显示。
- **三维预览**：贴图着色 / 骨骼 ID 染色 / UV 染色三种模式，线框叠加，点选部件高亮。
  DXT1/DXT3 DDS 纹理由 Rust 端解码（已与 PIL 逐像素比对，56 张全 0 差异）。
  手臂等条带 UV 环绕 u=0/1 接缝（引擎用重复寻址）：预览用 RepeatWrapping +
  跨缝三角形拆分副本顶点（UV ±1 归到缝同侧，编辑顶点时同步），否则三角形
  插值横穿整张纹理，手臂会出现乱纹色带；OBJ 导出对跨缝面追加调整后的 vt。
- **编辑**：
  - 材质：漫反射/环境光/自发光/高光色（含 alpha）、高光强度、纹理路径，实时生效；
  - 顶点：坐标 XYZ / UV 表格编辑，法线与骨骼绑定只读展示；
  - 文件：按当前顶点重算包围盒；
- **保存**：序列化器对全部 30 个 fskin 做过字节级往返测试（解析→序列化→与原文件逐字节相同），
  未修改字段（哈希、保留区、尾部填充）原样写回。
- **导出**：当前可见部件合并导出为 OBJ + MTL，纹理一并拷贝；蒙皮网格 + 动作一次
  带走的 glTF 全量导出已定案可行性，见下文「glTF 全量导出可行性」节。

### 运行

发布版（独立 exe，需 WebView2，Win10/11 自带）：

```
target\release\gongfu-studio.exe
```

开发模式：

```
run.bat                # 发布式：npm build 前端 → cargo build → 直跑 exe（嵌入 ui/dist）
run-dev.bat            # 开发式：vite dev (HMR) + cargo build --no-default-features → 运行
clean.bat / clean.sh   # 清理构建产物（target / ui/dist / src/wasm；--deps 连 node_modules 一起删）
npm --prefix gui/ui install    # 首次
npm --prefix gui/ui run tauri dev      # 开发运行（等价 run-dev.bat，由 tauri CLI 托管并监听 Rust 改动）
npm --prefix gui/ui run tauri build    # 生成 NSIS 安装包

npm run wasm:dev
```

dev 模式原理：`gui/Cargo.toml` 的 `[features]` 转发 `tauri/custom-protocol`——默认构建
（run.bat / tauri build）嵌入 ui/dist，无视 devUrl；`--no-default-features` 构建
（run-dev.bat / tauri dev）进入 cfg(dev)，窗口加载 tauri.conf.json 的 devUrl（vite，
端口 1420）。改前端 HMR 即时生效，改 Rust 关闭应用重跑脚本即可。

#### fmath：前端模型计算内核（wasm）

前端的重计算（CPU 蒙皮 LBS、裙摆去穿透、布料 verlet、锚定链矩阵、绑定矩阵
极分解拟合、镜像骨检测残差、FAK 顶点插值）在 `fmath` crate（wasm-bindgen），
`npm run build` / `npm run dev`（predev）自动经 wasm-pack 构建到 `gui/ui/src/wasm`
（产物不入库）。前端 `gui/ui/src/gfm.ts` 装载单例，`viewer.ts` 持 wasm 内存
视图直连 three.js BufferAttribute——每帧只上传一次世界矩阵平表，蒙皮/去穿透
结果零拷贝回写。数值口径与原 TS 实现对齐（f64 中间量）；cdp 采集（skirtRecord /
skirtPushLog / window.__viewer）接口不变。回退调试：URL 加 `?gfm=ts` 或
localStorage 置 `gfm_off=1` 后刷新，走 viewer.ts 内置 TS 实现做 A/B 对比。
前置工具链（run.bat / run-dev.bat / run.sh 的依赖步骤会自动自检，缺失时
`npm --prefix gui/ui run wasm-check` 给出指引）：全局 wasm-pack
（`cargo install wasm-pack`）+ rustup 的 wasm32-unknown-unknown 目标（自检
自动补装）。冒烟验证：`node gui/ui/scripts/wasm-smoke.mjs`（先
`npm run wasm`）；Rust 侧单测 `cargo test -p fmath`。

HMR 边界：改 `.vue` 组件 = 组件级热替换；改检查器 = 只重载检查器子树
（`components/InspectorPanel.vue`）；改 `viewer.ts` = **原位热替换，零重建**——
ViewerHost.vue 对 viewer.ts 做依赖级 accept，把存活实例的原型换到新类上，方法体 /
模块常量的修改即刻生效，视口、已加载模型、检查器、资源树全部不动（改构造器 /
类字段初始化不作用于已存在实例，需要时按 F5 整页重建）。改 App.vue 或 composable
仍是整窗重挂。

使用CDP来连接运行程序进行测试或者模拟点击

### 快捷操作

- 目录输入框回车或点「扫描」重新加载；
- 资源树/网格中带「角色」徽标的目录可右键「角色装扮」（`system` 目录等），
  文件菜单也有入口；启动时资源根能扫到角色目录则默认进入装扮模式；
- 装扮面板：基础模型按钮切换男女；套装 ‹ › 翻整套；各槽 ‹ › 逐件快切；
  帽子行可展开选挂点骨骼与 XYZ 偏移微调（对位不准时兜底）；
- 视口：左键旋转、右键平移、滚轮缩放，点击网格选中对应部件；
- 左侧卡片：开关 = 显示/隐藏，下拉 = 切换该组变体，点击卡片 = 选中编辑；
- 「保存当前部件」写回原文件；「另存为」可导出副本；红色 ● 表示有未保存修改。

### 帽子挂接（tag 挂点骨体系）

引擎约定（IDA 逆向 GongfuClient.exe + F3D.dll 确证）：角色 FSKEL 自带
`tag_Hat` / `tag_LWeapon` / `tag_LWrist` / `tag_RWeapon` / `tag_RWrist` 挂点骨；
客户端对 FSM 帽子调 `SGAvatarAttachment::BindToTagNode("tag_Hat")`，引擎每帧
`骨矩阵(tag) × 节点世界矩阵` 直接设为网格世界矩阵——无逐帽偏移、无去穿透，
顶点原样渲染（Hat 目录 78 个 FSM 全为刚体，长发型下段与衣领相交属游戏原貌）。
**骨骼点击拾取（2026-10-24）**：开骨架显示后点击骨线条/关节球——整段线条与关节
高亮变色、浮标显示 `#序号 骨名`（随骨骼投影跟随），点空处清除；检查器新增
「骨骼」折叠项列出全部骨名（支持过滤），列表点击与 3D 拾取双向联动高亮。

查看器等效实现：`viewer.ts` 挂 tag 骨（零偏移 + 内旋 Ry(π) + 锚点补偿
`TAG_MOUNT_OFFSET`，面板实测折算），无 tag 骨的骨架回落 Head01+固定偏移。

**长发 FSM 帽自动绑骨（2026-10-24）**：部分套装的长发在帽槽是静态 `.FSM`
（如旧包 FM38，492 顶点 Y 跨 87；引擎对 FSM 帽 = SGAvatarAttachment 刚挂，
客户端仅 tag_* 五挂点——原版同样刚体穿模）。查看器对 Y 跨 >50 的 FSM 帽件
自动转合成蒙皮件：顶点按 tag 挂接姿态入数据空间、**先对身体网格外推出
1.5 单位间隙**（原厂姿态贴背/嵌背；体内点射线奇偶判定推出+邻接松弛防皱）、
再最近分配到 头/颈骨+Bone* 发链骨（前二骨距离加权），走 LBS——近头随头、
发束随链动画（舞蹈大幅摆动）。
合成件 lp 按绑定帧生成、数据自洽，链诊断/锚定体系对其跳过（纯 LBS 即正确）。

**时装长发链锚定（2026-10-24）**：整体时装（shizhuang，如套装38 披发）与发型类
部件的 Bone* 链走**增量锚定**：`A_b(t) = 载体(t)·W₀(载)⁻¹·C·W_t(b)`（C = 链全部
影响点 帧0 LBS→authored 的链级刚体修正，残差>8 回退纯 LBS）——基准贴 authored
绑定价、链动画增量叠加。裙槽部件（Lower/Boot）保持纯 LBS 不变；错位链仍可经
失配>25 门槛锚定（qmw 系）。背景：时装资产普遍对非绑定姿态导出（FK38 裙链帧0
失配 12~17 与披发 12.6 同区间，数值不可分），裙的偏移在宽松裙容差内无害、长发
贴体穿模，故按部件语义分流。

装配层级（2026-10-24 IDA 定案）：一个 AvatarNode（.fskel + SkeletonAniCtrl，
world 由游戏逻辑显式设定）下挂两类子节点——`SGSkinNode`（蒙皮件：身体/头/
发/上衣/裤子等全部 .fskin，LBS 直接用父 AvatarNode 的骨矩阵，自身世界矩阵
原样转发父节点）与 `SGAvatarAttachment`（刚体挂件：帽/武器/特效，上式直挂）。
引擎无层级世界自动合成、无逐顶点去穿透；裤子等形变全部来自 FSKEL 烘焙 LBS
（含镜像骨修正），裙摆-腿穿插在原版即存在。

**动态加载问题与修复**（换装稳定性，两条）：

1. **挂接基准不确定**（2026-10-24 修）：挂接静态基准原取「点击瞬间的骨骼
   姿态」，动作播放中头部一直在动——不同时刻戴帽，日志锚/原点每次不同，
   且头部俯仰经 MODEL_YAW 共轭留残差，帽子贴合度随挂接时刻有细微差别。
   修复：骨架加载时快照帧 0（绑定姿态）全部骨矩阵 `bindWorldMats`，挂接
   恒以 bindW 为静态基准、以当前姿态播种相对运动——渲染结果成为与挂接
   时刻无关的帧函数（等价引擎逐帧重算），日志数值可复现（实测三个不同
   时刻挂接数值唯一）。
2. **快速切换竞态**（2026-10-23 修）：装备装配是异步的（CPK 读模型/贴图），
   来回切套装时上一次装配晚到仍挂入场景，卸载只按当前路径卸——残留
   「幽灵帽子」叠加错位。修复：`dressWants(path)` 守卫，装配完成即将挂入
   前校验该路径仍被开启槽位/时装需要。配套监控：挂接/卸下写诊断日志
   （绑定姿态锚/原点坐标），骨缺失与多帽叠加告警；CDP 直查
   `__viewer.boneAttachedInfo()`（锚/原点/包围盒/实时位置）与
   `__viewer.hatOverlapInfo()`（穿模顶点计数）。

### fskin 格式速查（逆向结论）

```
0x00 "SKIN" u32版本(0x100) u32顶点数 u32顶点偏移(0x138)
     1 u32索引区偏移 1 u32材质区偏移 f32包围盒[6] 保留256字节
顶点(变长): pos3 normal3 uv2 u32骨骼数 (骨骼u32 权重f32 骨骼局部坐标3)×n
索引区: 36字节小头(含两个哈希) u32三角形数 u16索引×3n 填充
材质区: f32 lead + diffuse/ambient/emissive/specular 各4f32 + 高光强度
        + 3字节前缀 + 纹理路径 + 尾部填充
```

同一目录的 `fskin.bt`（010 Editor 模板）与逆向分析报告见 `G:\pojie\ai\models`。

### FSKEL 动作（`gui/src/fskel.rs`）

主界面「动作」页签：扫描 `<模型目录>\act` 下的 `.FSKEL`（或单独打开），列出动作分段，
30 FPS 播放（播放/暂停、0.25×–2× 变速、逐帧拖动），可叠加皮肤网格对照绑定姿势。

- **BVH 导出**：全部帧或当前动作段，Blender 原生导入得到骨架 + 动作。
  关节旋转用 ZXY 欧拉通道；平移有动画的骨骼（根骨 / Spine01 脊椎拉伸 / 武器挂点）
  自动升级为 6 通道关节。
- **GLB 导出**：glTF 2.0 节点动画，四元数无损，每个动作一条命名动画片段
  （girl.FSKEL = 28 条），面向 three.js / 引擎管线。

导出数据均做过端到端回归：BVH 反解回 FK 世界坐标与源 FSKEL 最大偏差 < 0.005；
GLB 四元数/平移与源逐帧相等（0.0000°）。

#### fskel 格式速查（逆向结论，2026-10，294 文件 17209 帧全量验证）

```
0x00 "SKEL" u16版本(0x0100) u16校验 u32头大小(0x114) u32帧数 u32帧区偏移
0x14 [magic 0xB33BBD2E][01] 骨架对象内存映像(含 x86 指针残渣)
0x114 u32骨骼数; 0x118.. 字符串表 [u32长度(含\0)][骨骼名]×N
帧区 每帧(=一个完整姿态, 帧名重复动作名):
     [magic][01][f32帧间隔][u32][NUL动作名][u32骨数] [0][-1][1][1]根记录头
     + 整棵骨骼树(链式序列化) 直至下一帧
EOF  [magic][01][f32 1/30][u32 0]
节点记录 [u32 id][u32 父id][u32 链子数][链子id×链子数][T3][Q4][u8 镜像标志]:
     Q 恒 16B —— w=+0.0 时导出器存 4 字节 sentinel(0xB33BBD2E, 作 float ≈ -0.0002,
     引擎原值使用; 旧解析误当 [magic] 分隔符, 净推进字节相同所以姿态一直正确);
     镜像标志 1 = 引擎对该骨世界矩阵追加 Scale(-1,-1,-1)(IDA sub_1004C290 确证
     = 引擎骨记录 +52; 旧解析把 00/01 当"可选终止符/裸 01 分隔符"吞掉)
     girl.FSKEL 首帧镜像骨(跨动作文件稳定): tag_LWeapon/tag_LWrist、Bone37/33/43/
     26/55 链根、右手指 Nub×5、左趾 Nub; tag_LWeapon 标志帧间翻转(持械/空手)
残留链间分隔符: [magic][01] / 00
坐标系 Y-up，骨骼世界矩阵 = 父 × T × R(Q) × S(镜像骨)，30 FPS（帧数 = round(ActTime.csv毫秒×30/1000)，
与 ActTime.csv / girl.csv 完全对应）。AvatarGirl 骨架 = 3ds Max Biped（135 骨）+ Bone01-66
裙摆物理骨 + tag_* 挂点；girl.FSKEL 为主库（1633 帧 = std/wlk/run ×9 武具变体 + ze09）。
查看器镜像骨数据驱动（fskel.rs 解析标志位 → mirrorBones），残差推导仅作回退。
```

### glTF 全量导出可行性（蒙皮网格 + 动作，2026-10-26 定案，未实施）

现状两条导出路线各自缺一半：OBJ（`gui/src/obj.rs`）只有静态网格——合并导出对
每部件仅取首材质槽，MTL 无 alphaTest 等价物，镂空材质在编辑器里糊成实心；
FSKEL 的 GLB 导出（`fskel.rs export_glb`）只有骨架节点动画，不含皮肤/网格。
要把「网格 + 蒙皮 + 材质 + 贴图 + 动作」一次带进 Blender / three.js，glTF 2.0
是最贴合的目标格式——逐项映射基本直接，缺的只是写出器。

映射关系：

| 游戏数据 | glTF 对应 | 匹配度 |
|---|---|---|
| fskin 顶点 pos/normal/uv | POSITION / NORMAL / TEXCOORD_0 | 直接 |
| FSKEL 骨树（局部 T×R，父乘合成，Y-up） | nodes 层级 + TRS（glTF 同 Y-up、同序局部合成） | 直接 |
| FSKEL 动作分段 × 30FPS | 多个命名 animation，步长 1/30s（export_glb 已实现此半） | 直接 |
| 镜像骨 Scale(-1,-1,-1) | node.scale=(-1,-1,-1)；逐帧翻转的（tag_LWeapon 持械/空手）进动画 scale 通道 | 可表达 |
| mask 类材质 alphaTest（查看器口径 0.1） | `alphaMode:"MASK"` + `alphaCutoff:0.1` | 精确对应 |
| 多子网格多材质 | 每 submesh 一个 primitive、各配材质 | 优于 OBJ 合并导出（首槽问题的根治） |
| tag_Hat / tag_Weapon 挂点骨 | 节点树上的普通空节点，帽/武器可直接挂子节点 | 自然保留 |
| DDS 贴图 | 核心规范仅 PNG/JPG；`dds.rs` 已解 DXT1/3/5→RGBA8，补 PNG 编码即可 | 半成品 |

两个必须处理的技术点：

1. **蒙皮口径差异**：引擎每顶点存变长 influences，每个影响自带
   `(bone_index, weight, bone_local_pos)`（F3D 确证 Σ w·M·boneLocalPos）；
   glTF 是共享一个 POSITION + 每关节 inverseBindMatrices。两者在
   `bone_local_pos = 逆(绑定姿态骨世界矩阵) × pos` 时严格等价——单影响（刚性）
   顶点必然精确；混合顶点用 fskin.rs 现成的残差分桶（exact/approx）验证等价性，
   不等价的个别顶点取近似 IBM，代价为亚毫米级偏移。
2. **影响数截断**：glTF 常规蒙皮通道每顶点 4 影响（JOINTS_0/WEIGHTS_0），引擎
   为变长。统计全量 fskin 的 influences 上限，>4 的顶点取权重前四归一化；角色
   资产预期 ≤4，基本无损。

近似项与风险：固定管线材质→PBR metallic-roughness 只能近似（diffuse→
baseColorTexture 直接；高光强度 Ns→roughness 经验映射；ambient 丢失）；负
scale 骨骼在个别 DCC 里可能翻背面（Blender 无碍）；FAK 逐帧顶点快照若要带
动画须走 morph target（文件显著变大）或只导首帧静态网格。

实施判断：数据链路全部现成（fskel/fskin 解析、蒙皮矩阵计算、DDS 解码，GLB
容器与动画通道写法已在 export_glb 验证过），剩余工作 = 新增 `gui/src/gltf.rs`
（网格 primitive、skin 的 joints/weights/IBM、材质含 MASK、PNG 编码），量级与
obj.rs 相当。Blender 官方 glTF 插件对蒙皮 + 多动画导入成熟。

#### 装备部位拆分 vs「完整模型」（2026-10-26 架构对照定案）

现代游戏并未淘汰部位拆分——装备显示在 UE5 里本质仍是拆件组合：一个
`USkeleton` + 每槽位一个共享骨架的 `SkeletalMesh`，从属组件用 Copy Pose from
Mesh 复制主网格姿态；硬挂件（武器等）走 Socket（选骨 + 相对变换）；时装遮体
用 Section 显隐 / morph 收缩。「一张完整模型装下所有装备」在商业游戏里不存
在：男女 × 套装号 × 8 槽是组合爆炸，拆件 = 每件资产只存一份、运行时组合。

本作架构与 UE5 概念一一对应（部位拆分是换装系统在任何引擎的唯一可行解）：

| 本作（vEngine） | UE5 等价物 | glTF 表达 |
|---|---|---|
| AvatarNode（FSKEL 骨架驱动） | USkeleton + 主 SkeletalMeshComponent | nodes 骨树 + skin |
| SGSkinNode 共用父节点骨矩阵的 fskin | 从属 SkeletalMeshComponent + Copy Pose | 共用同一 skin 的多个 mesh |
| SGAvatarAttachment 刚挂 tag_Hat | Attach to Socket | 普通子节点挂网格 |
| 时装隐藏基础头/身体 | Section Hide / morph 收缩 | 节点显隐 |
| FSKEL 烘焙布料回放 | Chaos Cloth 实时布料 | 无（动画通道近似） |

glTF 单文件可装完整人物（一个骨架 + N 个部件 mesh 各带多材质 primitive +
全部动作 + 贴图），切装备 = 显隐对应 mesh 节点——正是上节导出方案的自然形
态。glTF 没有的：槽位语义、Socket 标准、布料/物理、LOD——装备组合逻辑永远
留在引擎侧。UE5 经 Interchange/glTF 导入器消费到**资产层**（蒙皮/动画/
morph 支持，PBR 材质映射良好），装配仍是引擎内蓝图/组件逻辑；Merge Actors
合并整网格是发布期优化而非创作形态。与 UE5 的差距不在拆分架构，在配套：
重定向 / 实时布料 / LOD / Nanite / 形变器，对应本作只有 FSKEL 烘焙回放。

### FAK 怪物顶点动画（`gui/src/fak.rs`）

资源网格 / 树中点击 `.FAK`（或对含 `.FAK` 的怪物目录「整体预览」）：逐帧顶点动画渲染，
按每帧自身时长播放（播放/暂停、0.25×–2× 变速、逐帧拖动、帧间插值），检查器中切换动作分段，
材质贴图（g080.dds / g080+.dds）按材质路径在同目录自动匹配。G080 = 821 顶点 925 三角，
4 子网格 2×2 材质，133 帧 = atck20/spck28/bek7/std20/walk12/dead46。

#### fak 格式速查（逆向结论，2026-10，G080.FAK 3.5MB 全字节验证，往返逐字节一致）

```
0x00 "AKK\0" u32版本 u32顶点数 u32子网格数 u32子网格区偏移(恒0x13C)
     u32材质数 u32材质区偏移 u32帧数 u32帧区偏移 f32包围盒[max3,min3] 保留256字节
子网格区 ×N: [u16 0][u16 序号][u16 0][u16 顶点数][f32包围盒6][u32三角形数]
             [u16索引×3n][20字节0填充]    — 四块索引对顶点池构成不相交划分
帧区 ×N: [动作名4B][f32帧时长][u32顶点数][顶点记录32B ×N]
         首帧前无分隔符，其后每帧前 [u32 0]，区末 [u32 0] 终止
         顶点记录: [pos3][normal3][uv2] — 法线未归一化(长度2.1~3.0)，UV 全帧恒定
材质区 ×N: 等分至文件尾，每条: [u32序号][diffuse/ambient/emissive/specular各4f32]
           [f32高光][3字节前缀][NUL路径][0填充]   — 布局与 fskin 材质相同
帧时长: 多数 0.125s（各动作末帧 1/12 或 1/24），dead 全 1/30；同名连续帧 = 动作分段
（G080.csv 的 atck 1.3093s 与内嵌帧时长之和 2.458s 不一致，疑为引擎按 CSV 变速播放）
```

### FSCN 场景静态网格（`gui/src/fscn.rs`）

建筑 / 迷宫 / 道具模型的场景网格（`Building/`、`Maze/`、`ItemSys/ItemMdl/`，system.cpk
全量 1454 个）。SCN = FSM 头部多一个 BVH 段偏移字段 + 尾部多一棵包围体层次（BVH，
疑似用于场景剔除/拾取）；顶点 / 子网格 / 材质 / 挂点四区与 FSM 逐字节同构。
查看器：资源网格点击 `.FSCN` 走 FSM 同款渲染路径（结构兼容，挂点恒空），检查器
显示网格统计 / BVH 记录数 / 材质贴图（可热替换）；资源树附带轻量统计
（顶点/三角形数，走头部+子网格区，不整读 BVH）。

贴图注意（与 FAK/FSM 模型目录自包含不同）：建筑贴图在材质引用的
`system\building\texture\`（包内 2121 个文件，且全为 `.dds`——引用却按 `.tga`
书写）；道具模型（ItemSys\ItemMdl）贴图则与模型同目录。解析口径（前端
`resolveModelTextures`，全量引用统计 5094 条 .tga / 162 条空路径）：
1. 路径优先：引用目录按模型路径祖先（CPK 根）拼接，原名未命中换 `.dds` 重试
   （G01DW01 4/4 命中）；换图下拉在模型目录无贴图时改列引用贴图目录；
2. 同目录 stem 回落（WK11 引用 system\itemmdl\ 不存在，同目录 wk11.dds 命中）。
未命中约 249 条为空路径材质（本就无贴图）与原始数据笔误（GO2A 字母 O、
G09B-016 串号、fukongshi 引用 building 根目录）。

#### fscn 格式速查（逆向结论，2026-10，system.cpk 全量 1454 文件解析+往返 0 失败）

```
0x00 "SCN\0" u32版本(恒1) u32?(恒1) u32顶点数 u32顶点区偏移(恒0x148)
     u32子网格数 u32子网格区偏移 u32材质数 u32材质区偏移 u32挂点数(恒0) u32挂点区偏移
     f32包围盒[min3,max3] 保留256字节0 u32 BVH段偏移 = 挂点区偏移+4+挂点数×36
     （FSM 顶点从 0x144 起；SCN 多这个树偏移字段，顶点后移到 0x148）
顶点区 ×N×32B: [pos3][normal3][uv2] — 法线未归一化；无贴图顶点 UV 原生
              0xFFFFFFFF/NaN（G06DW07-A 有 88 个，非损坏）
子网格区 ×N: 与 FSM/FAK 同款变长记录（61/1454 分段池 base≠0、47 序号≠位置共材质）
材质区 ×N: 等分至挂点区，恒 0x46C/条 — 与 FSM/FSKIN 材质槽同宽同布局
挂点区: [u32 数量=0]（全量样本无挂点记录，仅剩数量字段）
BVH段（SCN 特有）: [u32 记录总数][记录×N]，第 3 个 u32 为 0 是内部节点、>0 是叶子
  内部节点 36B: [u32左子][u32右子][u32 0][bbox max3,min3] — 子为记录数组全局
                下标，0xFFFFFFFF=空侧（289/1454 样本出现）
  叶子: [u32 -1][u32 -1][u32三角形数][u16顶点索引×3n][bbox 6f32] — 索引为
        顶点池全局下标（无子网格基地址叠加）
根为记录 0；满二叉时内部节点在前叶子在后（G01DW01: 262+263=525）；
全部叶子三角形数之和 == 子网格三角形总数（全量样本逐字节核对）
```

### SFX 特效脚本（`gui/src/sfx.rs`）

战斗 / 场景 / 角色特效的节点脚本（`Effect/Combat|Scn|role|…`，system.cpk 全量
786 个）。文件为「根变换 + 顺序节点表 + 尾部路径数组」，11 种节点类型：
EffectQuad（公告板）、EffectParticleEmitter（粒子）、EffectCross（交叉面）、
EffectCylinder（柱面）、EffectBeam（光束）、EffectGrid（面片）、EffectMesh
（FAK 网格）、EffectSphere（球面）、EffectMsg（数值飘字）、EffectExplosive
（爆炸）、EffectSound（音效）。查看器（2026-10-27 起按引擎真实形态渲染）：
Quad/未知类型 = 相机朝向公告板，**尺寸/颜色取状态关键帧「长成后」值**（引擎
通道 {RGBA,半宽,半高}，首态常为 0 生长、末态已淡出，故取尺寸最大帧）；Cross
= 两片垂直交叉面片（prop+140 非 0 追加**水平第三片**地贴模式）；Cylinder =
**程序管网格**（21 通道：环绕圈数/底顶半径/花边/锯齿/螺旋抬升/上下 RGBA 顶点
色，prop+112 分段数 +128 UV 平铺，块相对 raw 错 4B）；节点按 prop 偏移真实
摆放，挂路径名的节点做 **EffectMotion 路径动画**（速度关键帧插值 + 逐帧积分，
与引擎 EffectMotion::Update 同语义）；**U×V 帧网格贴图按引擎参数翻页**（子矩形
步进，帧率/停帧位来自尾块 +268/+272，播放时间轴为秒、长度 = 状态总时长），
叠加混合（可切普通透明）；根矩阵线性部分作用于整组、
世界平移丢弃（Combat 平移为编辑器残留、Scn 摆放位离原点数千米）。检查器：
节点列表限高滚动，每节点**显隐切换**（公告板+标签一并隐藏），元信息含
偏移与挂接路径名；贴图缩略图点击放大（吸底固定高度常驻）并可**改写贴图
引用路径**（raw 内原槽位就地覆写、长度只减不增，同目录下拉 + 自定义路径，
未解析到 .tga 时可改指 .dds），改写经「保存」落盘（CPK 内不可写）；
EffectSound 引用可在线试听。资源树附带轻量统计（节点数）。贴图在
`system\effect\tex\`，与 FSCN 同「路径优先」口径解析（祖先拼接 + .dds 重试）。

节点参数区 = prop 定长块（Quad 184B / Cross 208B / Motion 124B，头 36B 即
[u32 节点号][32B 名字缓冲]）+ [u32 N][N×状态关键帧]（Quad 72B/Cross 124B/
Motion 60B）+ 340B EffectBase 公共尾块 = tagMtlEffectDesc（2026-10-27 IDA
实测 CreateBasicMtl/CreateMtl/MMUVTrans：+0 起**贴图路径 256B 缓冲**（空 =
不建材质即不可见）、+256 u8 混合预设 1..4（MMFrameBlend 两参数组
{4,5}/{4,1}/{0,2}/{9,0}）、+257 u8 深度修改器开关（0=挂 MMDepthFrame）、
+260 u32 U 帧数、+264 u32 V 帧数（SetScale(1/u,1/v) 确证网格分帧）、
+268 f32 翻页帧率（帧时长=1/fps）、+272 u8 播完停帧（1=停末帧/0=回绕），
其余恒零；**无任何显隐位**）。Quad/Cross
的 prop+112 = 位置偏移 vec3（公告板世界位 = 偏移 + 路径积分位置，F3D.dll
SetOffset/Update 实测）、prop+80 = 32B 挂接路径名（"none" = 定点）；其余
类型 prop 布局未逆向（偏移按 0）。尾部路径数组 = [u32 M][M×EffectMotion]
（引擎 SGEffectNode::Deserialize 第二个对象数组，AddEffectPath 挂接；
状态 = {时长, 速度 xyz, 角速度 xyz}，looped 回绕），786 文件实测恒
EffectMotion、M=0..22。节点边界按「下一个 [u32 长度][已知类型名]」定位
（末节点收于尾部路径数组起点，链式校验恰好收于文件尾），参数字节原样
保留（保存 = 字节级还原）。参数区常见 0xCD 未初始化填充，系 MSVC 调试版
导出的原貌。

#### sfx 格式速查（逆向结论，2026-10，system.cpk 全量 786 文件解析+往返 0 失败）

```
0x00  f32×16 根变换矩阵（行主序，末行为平移，[3][3] 恒 1.0；Scn 场景特效
      为世界坐标摆放位，Combat 特效平移为编辑器残留——引擎挂接走
      SetWorldMat 整体替换）
0x40  u32 节点数 N
0x44  节点记录 ×N 顺序排列:
        [u32 类型名长(含NUL)][类型名 NUL][u32 节点号][节点名 32B 缓冲 GBK
         NUL 填充][类型参数变长区…]
      节点号 = 节点表位置（EffectSound 恒写 0）；节点名为设计师命名
      （"lujing"/"光传递_3"/"dust_a"，全量最长 15 字节）
尾部  [u32 路径数 M][M × EffectMotion 记录]（结构同节点记录，name 即
      路径名，供节点 prop+80 引用；M 可为 0——仅 4 字节计数）
类型参数区内的资源引用（NUL 结尾可打印串，全小写前缀）:
      system\effect\tex\…\*.tga  贴图（9802 条；2026-10-26 全量复扫更正：
      每节点恰 1 条——9802 条引用 == 9802 个有贴图节点，多贴图节点 0 个，
      早前「多贴图 = 逐帧动画序列」「帧序列在 tex 目录相邻文件」两说均不
      成立；帧动画在**单 tga 内 U×V 网格**，由 EffectBase 尾块 +260/+264
      网格 +268 帧率 +272 停帧位驱动引擎 MMUVTrans（RTTI 名 MMUVPage）播放）
      system\…\*.fak             EffectMesh 网格引用（111 条）
      sound\…\*.wav              EffectSound 音效引用（281 条）
```

