# CDP 渲染数据提取工具集

通过 Chrome DevTools Protocol 连接运行中的 Gongfu Studio（Tauri 2 / WebView2），
在不改动程序的前提下驱动 UI、读取 viewer 内部渲染数据（蒙皮后顶点、骨骼矩阵、
碰撞胶囊、锚定/镜像状态、诊断日志）、抓取 WebGL 画面。穿模/裙摆/蒙皮类问题的
排查与数值验证首选此链路——截图受视觉模型噪声影响，数值结论以此为准。

## 1. 启用 CDP

- 端口用 **9333**。`9222` 落在本机 Windows 保留端口段（9181-9280，Hyper-V/WSL
  动态保留），参数出现在命令行但端口静默不监听。查询：
  `netsh interface ipv4 show excludedportrange protocol=tcp`。
- 本机已写入 HKCU 策略注册表（持久，双击 exe / run.bat 启动均生效）：
  `HKCU\Software\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments`
  下字符串值 `Gongfu-Studio.exe` = `--remote-debugging-port=9333`。撤销即删该值。
- 一次性启用（PowerShell）：
  `$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9333'; Start-Process target\debug\Gongfu-Studio.exe`
  （Git Bash 里 `VAR=... cmd //c start` 会拒绝访问，必须 PowerShell。）
- 重启应用时先 `taskkill //IM Gongfu-Studio.exe //F` **并杀掉旧 msedgewebview2 浏览器
  进程**，否则旧浏览器进程被复用、调试参数不生效（表现为端口不监听）。
  Git Bash 里 `/IM` 会被转义成路径，须写 `//IM`。
- 就绪判定：`curl -s http://127.0.0.1:9333/json`。target 选择必须**优先
  `tauri.localhost`**——打开过 DevTools 后 `devtools://` 页面也是 type=page，会抢首个匹配。
- **改了前端（gui/ui）后**：Tauri 在编译期把 `ui/dist` 嵌进 exe，只跑 vite build 不够，
  必须 `npm --prefix gui/ui run build && cargo build` 后重启 exe；exe 被运行实例锁住时
  链接报 os error 5，先杀进程。（dev 模式除外，见下条）
- **dev 模式（run-dev.bat / tauri dev）**：窗口加载 vite（`http://localhost:1420`），
  改前端 HMR 即时生效，无需 vite build + cargo build，仅 Rust 改动需重启（重跑脚本）。
  CDP target URL 相应变为 `localhost:1420`。

依赖：`cdp` 目录下 `npm install`（ws；本机 npm 代理若失效加 `--noproxy "*"`）。
CDP_PORT 环境变量可覆盖默认端口。

## 2. 工具清单

| 命令 | 用途 |
|------|------|
| `node drive.mjs "<js 表达式>"` | 页面内一次性求值并打印（自动 await Promise，可返回 JSON） |
| `node drive.mjs --diag [N]` | 取 `window.__diagLog` 环形缓冲（500 条）最近 N 条（默认 100） |
| `node drive.mjs --tail` | 实时流页面 console 输出（Runtime.consoleAPICalled） |
| `node drive.mjs --shot <file>` | Page.captureScreenshot 整页截图，不受窗口遮挡影响 |
| `node grab-frame.mjs <file.png>` | 在 rAF 回调里 `canvas.toDataURL` 抓 WebGL 帧存 PNG |
| `node verify-skirt.mjs` | 采样当前帧全部裙顶点 vs 腿胶囊，输出残余穿透（一键复查穿模） |
| `node probe-frontflap.mjs` | 逐布料链对比 纯LBS vs 锚定摆放 的穿透 + 失配窗口分布（锚定类问题首选） |
| `node probe-hipclip.mjs [f\|f0-f1]` | 裙顶点 vs 身体网格三角形（腿/骨盆分区）最近距离 + 胶囊间隙对照（覆盖缺口定位） |
| `node probe-triclip.mjs [f0-f1]` | girl_Lower 内 裙三角形×身体三角形 相交逐帧统计（面片跨穿） |
| `node probe-xclip.mjs [f0-f1]` | 全部件全组合 三角形相交（含软×软：裙自折/裙×上衣/发×身）——穿模定位终极探针 |
| `node analyze-tail.mjs <log>` | 合并分析 `--tail` 抓的日志文件，按动作分组统计告警 |

新写脚本直接复制 drive.mjs 的骨架（getJson 拿 target → ws → Runtime.evaluate）即可。

判读三角形相交结果时注意：服装网格存在大量**设计内恒定重叠**（双层布料、
靴套腿管、腰带压裙顶、腿管入腰带），表现为逐帧常数背景；真正的穿模是
**随帧变化**的相交对（如 run9 474-477 的 strip×leg）。tri-tri 检测器的
点在三角形内判定对绕向敏感——复制这些脚本时先用合成三角形自测再信结果。

## 3. `window.__viewer` 后门（数据入口）

viewer 构造器把自身挂到 `window.__viewer`。TS `private` 只是编译期标记，
**运行时全部字段/方法可直接访问**。常用成员速查（字段随帧更新，读取即当前渲染状态）：

| 成员 | 内容 |
|------|------|
| `parts` | Map<路径, PartObj>。`p.geom.getAttribute("position")` = **蒙皮+去穿透后的顶点位置**（Float32Array, xyz）；`p.model.vertices[i]` = 原始数据 `{pos, normal, influences}`（authored 绑定位）；`p.inflOf[i]` = 与几何顶点一一对应的影响表（**含 UV 拆缝克隆**，克隆源 = `p.clones[i - vcount].src`） |
| Influence | `{boneIndex, weight, boneLocalPos:[x,y,z]}`；引擎 LBS = Σ w·M·boneLocalPos，原始权重不归一化 |
| `worldMats` | 当前帧各骨世界矩阵（THREE.Matrix4；`.elements` 列主序 16 浮点，M·v 用 `e[0]*x+e[4]*y+e[8]*z+e[12]` 三行） |
| `worldsFromPoses(frame)` | 方法：按 FSKEL 原始姿态（含镜像）算世界矩阵数组——做"纯 LBS 对照"时用它，不受渲染期锚定/模拟覆盖影响 |
| `fskelPoses` | Float32Array 原始帧数据：每骨 7 浮点 = 平移 xyz + 四元数 xyzw，帧步长 = `fskelBoneCount*7`，帧号 f 偏移 `f*stride + b*7` |
| `skelBones` / `skelParents` / `fskelBoneCount` | 骨名表（`.name`，Bip01 系身体骨 + Bone01-66 布料骨）/ 父索引 / 骨数 |
| `legCaps` | 腿碰撞胶囊 `{bone, len}[]`。胶囊端点：a = worldMats[bone] 平移，b = a + 矩阵 X 轴 × len，半径 r = 0.18×len |
| `skirtVerts` | 受 Bone* 链影响的裙片几何顶点 `{p, i}[]`（i 为 inflOf 下标，含拆缝副本）——去穿透的作用集 |
| `anchoredRoots` / `anchoredChains` / `anchoredMats` | 锚定链判定结果 / `{carrier, tpInv, bindOf}` 拟合数据 / 每帧合成矩阵 |
| `mirrorBones` | 镜像骨索引集合（Scale(-1,-1,-1)） |
| `isBoneHelperName(b)` / `boneChainMembers(root)` | 方法：Bone* 判定 / 链成员（根+子孙骨索引） |
| `applyFskelFrame(f, f, 0)` | 方法：**精确设帧**（暂停后调用即定格到 f，播放循环不会覆盖） |
| `diag` | 当前 30 帧窗口累计：`i0`(帧号) `maxNeed/maxBone`(去穿透需求) `clamp`(钳制) 等 |
| `actName` | 当前动作名（诊断日志前缀） |

诊断日志同时写 `window.__diagLog`（环形 500 条，元素 `{t, level, msg}`），
历史用 `--diag`，实时用 `--tail`；CDP 没有 console 历史 API，别指望回捞。

## 4. 常用配方

**残余穿透复查**（任何动作）：`node verify-skirt.mjs`
→ `{"residualPen": -0.46, "badOver2cm": 0, ...}`（负值 = 全在胶囊安全间隙外）。

**精确定格某帧再读数据**：
```js
v.applyFskelFrame(40, 40, 0)   // 定格帧 40；随后读 parts/worldMats 即该帧状态
```

**读蒙皮后某顶点位置 / 全链对照分析**：复制 probe-frontflap.mjs 的骨架——
注意页面**没有全局 THREE**（打包产物），矩阵运算需自带列主序 4x4 乘法
（该脚本里有现成 `mul` 实现可抄）。

**抓 WebGL 画面**：three.js 默认 `preserveDrawingBuffer:false`，直接
`toDataURL` 是空白；必须在 `requestAnimationFrame` 回调里（注册晚于应用
渲染循环，同帧内其后执行）取（grab-frame.mjs）。整页/UI 截图用
`drive.mjs --shot`。PowerShell CopyFromScreen 式窗口截图会被桌面遮挡污染，弃用。

## 5. UI 驱动（无 Vue store 访问，只能走 DOM）

- **切动作**：检查器"动作"折叠面板里的 `.action-row`（按 `.innerText.includes("动作名")`
  找到后派发 mousedown/mouseup/click 三件套）。
- **切动作文件**：`.act-panel` 里的 n-select。naive-ui 下拉是**虚拟列表**，
  需把滚动容器（`.n-virtual-list` 或 `.n-base-select-menu .n-scrollbar-container`）
  从 `scrollTop=0` 起步 `+=200` 逐步滚动探测，目标选项出现在 DOM 再点
  （probe 用法见会话脚本；直接 querySelector 找不到未渲染的选项）。
- 点击类操作一律派发真实事件序列（mousedown → mouseup → click，bubbles:true），
  仅 `el.click()` 对部分组件不生效。

## 6. 坑汇总

- drive.mjs 的表达式经 bash argv 传递：**反斜杠会被转义链吃掉**，JS 字符串里
  避免 `\\`（用 `k.slice(-n)`、`o.innerText` 等替代路径切分）；复杂逻辑宁可落成
  .mjs 文件。
- CDP `Runtime.evaluate` 返回大对象用 `returnByValue:true` + 先在页面内
  `JSON.stringify`（或返回 Promise，`awaitPromise:true`）。
- 相对资源路径（`.\res\...`）以 exe 工作目录解析，启动时给对 WorkingDirectory
  （仓库根）；应用启动会自动恢复上次会话的部件与动作。
- 高刷屏下 `applyFskelFrame` 同帧可能被多次调用（诊断窗口计数口径），以
  `diag.i0` 为准。
