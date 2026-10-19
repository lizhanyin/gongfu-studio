# Gongfu-studio

功夫世界（软星 vEngine 引擎）游戏资源的逆向工程与 Mod 工具集。

| 目录 | 说明 |
|------|------|
| `cpk/` | CPK 资源包格式（`RST\x1A`）读写库，Cpack2009 官方打包器（CPKCore 2.0）的 Rust 移植 |
| `cpktool/` | 命令行工具：查看 / 列表 / 解包 / 打包 / 添加 / 删除 / 校验 |
| `gui/` | Gongfu Studio（见下节），从 gongfu-studio 迁入的 Tauri 2 + Vue 3 GUI；前端在 `gui/ui`，Rust 端在 `gui/src` |

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

`.fskin`（SKIN v1.0）游戏模型文件的预览与编辑器，含 `.fskel`（SKEL v1.0）骨架动画预览与导出，
Tauri 2 + Vue 3 + Naive UI + three.js。

### 功能

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
- **导出**：当前可见部件合并导出为 OBJ + MTL，纹理一并拷贝。

### 运行

发布版（独立 exe，需 WebView2，Win10/11 自带）：

```
target\release\gongfu-studio.exe
```

开发模式：

```
npm --prefix gui/ui install    # 首次
npm --prefix gui/ui run build  # 构建前端（gui/ui/dist）
npm --prefix gui/ui run tauri dev      # 开发运行
npm --prefix gui/ui run tauri build    # 生成 NSIS 安装包
```

### 快捷操作

- 目录输入框回车或点「扫描」重新加载；
- 视口：左键旋转、右键平移、滚轮缩放，点击网格选中对应部件；
- 左侧卡片：开关 = 显示/隐藏，下拉 = 切换该组变体，点击卡片 = 选中编辑；
- 「保存当前部件」写回原文件；「另存为」可导出副本；红色 ● 表示有未保存修改。

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
节点记录 [u32 id][u32 父id][u32 子数]:
     子数=0: [T3][Qxyz]               — Qw=+0.0 时省略
     子数≥1: [u32链子id][子数-1×附加子链根][T3][Q4][u8 00 可选]
链间分隔符: [magic][01] / 裸 01 / f32 1.0 / 00
坐标系 Y-up，骨骼世界矩阵 = 父 × T × R(Q)，30 FPS（帧数 = round(ActTime.csv毫秒×30/1000)，
与 ActTime.csv / girl.csv 完全对应）。AvatarGirl 骨架 = 3ds Max Biped（135 骨）+ Bone01-66
裙摆物理骨 + tag_* 挂点；girl.FSKEL 为主库（1633 帧 = std/wlk/run ×9 武具变体 + ze09）。
```
