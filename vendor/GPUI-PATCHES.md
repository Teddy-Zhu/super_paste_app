# GPUI 0.3.11 本地补丁

本轮原生 UI 调试涉及的补丁：

- Base `VirtualList::with_fixed_item_size` 允许应用提供已知卡片尺寸，跳过调用渲染器测量单项的路径；其他调用者继续沿用原测量方式。浮层的尺寸索引、测量和实际卡片容器共用尺寸计算，避免把测量范围当作可见范围，反复回收、重载预览并使首帧准备无法完成。
- SVG 绘制在叶子未指定颜色时使用窗口当前继承的文字颜色，修复按钮、搜索和空状态图标不绘制；内存、外部文件和资源路径共用此规则。
- X11 PopUp 设置 `_KDE_NET_WM_SKIP_CLOSE_ANIMATION`，由应用动画负责退出，避免 KWin 再次处理隐藏窗口的旧纹理。
- X11 override-redirect 浮层直接置顶、取得输入焦点，并以 FocusIn/FocusOut 更新激活状态；普通受窗口管理器管理的窗口保留 EWMH 激活路径。上游针对普通窗口的 2px 横移绕过不再应用于 PopUp，恢复工作区边缘定位。
- X11 焦点处理忽略键盘抓取及释放产生的 `Grab` / `Ungrab` 临时通知，避免全局快捷键被误判为失焦，导致面板先关闭又重新打开；真实焦点切换仍更新激活状态和输入法焦点。

- 聚焦的 Base 输入控件在完成布局后刷新原生候选框坐标，避免初次聚焦和动画布局沿用旧坐标；失焦窗口不提交候选框坐标。浮层折叠搜索仍保留不可见的非零输入区域。X11 输入上下文复用、成对请求预编辑与状态样式，并显式接入焦点进入/退出；连接尚未完成时不发送上下文操作。当前本机 Fcitx 使用外置预编辑，候选框跟随搜索区域，选词提交后再展开搜索，不声称已验证所有输入法模式。

本轮在 KDE X11 的开发程序中实际检查了面板和设置布局；原生窗口行为尚未覆盖 Windows、macOS 和 Wayland。旧发布包不包含本轮变更。

严格文档绘制新增 `TextLayout::try_paint_foreground`，使原生字形与装饰绘制错误返回文档层；普通控件保持原有记录错误的行为。Kit 正文片段及列表标记共用所属文档代次的 `NativeLayoutGuard`，失败后使用已有提示与选择清理流程，同代只通知一次；空列表项也能报告标记错误。沿用既有布局和字形绘制结果，不预先栅格化、不新增缓存、任务或转换。Linux 桌面类型检查通过（`target/native-paint-feedback-check-01.json`）；未运行 UI；随后已纳入 Linux 发布及四种交付包刷新 17、Windows GNU 全目标检查 16。macOS 桌面检查 02 受 Apple SDK 缺失阻断，不能据此声称该平台通过。

角度类型、JSON schema 与 Cosmic 属性按 CSS Fonts 4 接受 ±90° 端点，允许请求进入原生字体面及 slnt 轴匹配；原生轴仍使用夹紧后的共享实例。Linux/Windows 合成的角度换算改用双精度，Apple 已沿用该精度；没有新增缓存。仅合成的 ±90° 无有限栅格表示，Linux 的 GPUI 栅格入口、Cosmic 的图片/轮廓入口及 Windows 的 DirectWrite 字形分析入口在分配前拒绝；其他大剪切沿用既有像素预算。详情适配器不再仅因端点类型校验拒绝整份文档。Linux 桌面构建 107、Cosmic 3 项及 GPUI 2 项无窗口回归、Windows GNU 全部目标检查 14 和 Apple/macOS 后端库检查 06 通过；后者不含桌面二进制。Windows/Apple 原生运行及 UI 尚待验收，该轮源码已纳入 Linux 交付刷新 16。

来源为 crates.io 的 `gpui-pre`、`gpui-pre-linux`、`gpui-pre-macos`、
`gpui-pre-windows`、`gpui-pre-apple`、`gpui-pre-wgpu` 0.3.11。上游 Zed 源码修订由各包
`package.metadata.gpui-pre` 记录。许可证为 Apache-2.0；保留各包原有版权、
声明和资源，完整许可文本在 `../licenses/Apache-2.0.txt`。

补丁服务应用原生窗口生命周期、快捷键录制、共享像素绘制和文字排版；Kit 仍使用固定的 0.9.0 系列，不引入 WebView。

- Apple 构建脚本按 Cargo 提供的 `CARGO_CFG_TARGET_OS` 选择目标资源生成，`cbindgen` 使用宿主构建依赖，修复 Linux 宿主交叉检查时漏生成真实着色器及头文件的问题。既有 `runtime_shaders` 分支继续拼接原始 MSL 与生成头文件，未引入占位资源；其他分支仍要求原生 Metal 工具链。宿主与目标区分依据 [Cargo 构建脚本文档](https://doc.rust-lang.org/cargo/reference/build-scripts.html)。macOS 窗口三处 `Option::map_or_default` 改用等价稳定的 `map_or_else(Default::default, ...)`，兼容项目 Rust 1.95。Apple 文字/绘制及 macOS 窗口后端库已通过真实 `x86_64-apple-darwin` 交叉类型检查（`target/apple-cross-check-05.*`）；未检查桌面二进制或原生链接。测试目标因现有 GPUI `test-support` 特性未启用而未编译通过（`target/apple-cross-check-03.jsonl`），测试运行及实际显示仍待原生环境验证。

- Windows 作者倾斜角随来源样式进入匹配、原生运行段与既有实例身份。真实 slnt 范围或真实斜体沿用实际实例；其余直立字体先移除 DirectWrite 固定 OBLIQUE 模拟并保留 BOLD，再合成请求角度。原始字体面继续独立持有，绘制面仅在移除模拟时另持原生引用；没有新增字体字节或并列缓存。度量边界、装饰墨迹、灰度/子像素与既有 COLR 栅格化使用同一剪切，基线及图集原点作逆变换补偿，水平 advance 不变。大剪切限制单维 8192px、RGBA 16MiB，超限报错；未扩展原有彩色字形格式支持。详情已开放 Windows 合法角度，±90° 可进入原生面和轴匹配，仅纯合成端点拒绝栅格化；Apple 接入见下述补丁。现有非 UI 后端补充正负角度、来源/回退上下文、墨迹、像素及预算断言，GNU 桌面与后端全部目标交叉检查通过（`target/windows-cross-check-12.log`），原生用例未运行。依据 [CreateFontFaceWithSimulations](https://learn.microsoft.com/en-us/windows/win32/api/dwrite_3/nf-dwrite_3-idwritefontfacereference-createfontfacewithsimulations)、[IDWriteFontResource::CreateFontFace](https://learn.microsoft.com/en-us/windows/win32/api/dwrite_3/nf-dwrite_3-idwritefontresource-createfontface) 与 [CreateGlyphRunAnalysis 变换及基线语义](https://learn.microsoft.com/en-us/windows/win32/api/dwrite/nf-dwrite-idwritefactory-createglyphrunanalysis)。

- Windows 请求端使用 `IDWriteFontSet1::GetMatchingFonts` 按浮点 wght 和原生宽度、ital/slnt 匹配，直接创建所选实例，避免将连续字重先压成整数 WSS。实际坐标继续走既有度量、塑形与栅格化链，不增加扫描或缓存；仅 E_NOINTERFACE 保留旧系统的 WSS 能力，其他失败不降级。回退配置构建失败使本次选择失败，不再作为空配置缓存。非 UI 后端回归检查实际请求的分数字重及再次塑形，GNU 桌面及后端各目标编译检查通过，原生用例未运行；作者自定义角度的完整链见上述补丁。实例化与排序语义依据 [IDWriteFontSet1::GetMatchingFonts](https://learn.microsoft.com/en-us/windows/win32/api/dwrite_3/nf-dwrite_3-idwritefontset1-getmatchingfonts)。

- Windows 保留 DirectWrite 塑形实际返回的字体面及其 COM 生命周期，已有字体索引按原生身份、来源特性和回退链区分，不再按家族/字重重新选择默认实例。原生轴值进入字体引用、单字体集合和 TextLayout4，实际回退字号传入绘制；仅归一既有断连字用的一 ULP 字号微调。单一来源运行段不分配边界表，多段沿用 UTF-8/UTF-16 转换并查找来源上下文；不增加字体字节副本或并列缓存。当前语言及 en-US 均缺失时读取实际首个家族名，空列表报错。非 UI 后端用例覆盖分数字重轴、实际字体面、来源配置隔离与再次塑形；Windows GNU 桌面与后端测试目标编译通过，未运行原生用例。作者自定义倾斜角链见上述补丁。接口依据 [IDWriteFontFace5](https://learn.microsoft.com/en-us/windows/win32/api/dwrite_3/nn-dwrite_3-idwritefontface5)、[字体面引用](https://learn.microsoft.com/en-us/windows/win32/api/dwrite_3/nn-dwrite_3-idwritefontfacereference1) 及 [DirectWrite 可变字体](https://learn.microsoft.com/en-us/windows/win32/directwrite/opentype-variable-fonts)，已对照固定版本绑定。

- Apple 来源上下文现包含请求样式与完整浮点字重。家族候选按原生 `wght` 可达范围匹配，选中实例在塑形前写入请求值；原生轴夹紧不合并逻辑请求身份。原生 variation 字典先创建独立可变副本，再覆盖目标轴，避免替换嵌套字典时丢失 `opsz` 等其他坐标；字体特性、级联、字号和矩阵继续保留。合成主字体的实际回退通过 [CTFontCreateCopyWithSymbolicTraits](https://developer.apple.com/documentation/coretext/ctfontcreatecopywithsymbolictraits(_:_:_:_:_:)) 优先选择同家族真实斜体及粗体；面或轴改变时沿原 UTF-16 范围最多再塑形一次。已有真实样式实例直接复用索引；缺少真实粗体时以同一 `1/36 em` 水平偏移处理度量、墨迹与像素，不改变 advance；彩色字体及已达到粗体的实例不叠加。偏移比例参考 [WebKit Core Text 后端](https://github.com/WebKit/WebKit/blob/main/Source/WebCore/platform/graphics/coretext/FontCoreText.cpp)，这里自行实现绘制与几何衔接。原生墨迹路径只提取一次，投影原路径及平移副本；元素遍历适配固定依赖 `zed-font-kit 0.14.1-zed` 的 Core Text loader，保留 Pathfinder 版权并选用 Apache-2.0。没有新增字体字节副本或并列缓存。新增原生非 UI 回归源码覆盖分数字重、端点、非默认光学轴、回退及真实/合成粗斜体；后端库交叉类型检查已通过，测试目标与原生运行限制见上述构建补丁。

- Apple 保留完整来源 `FontStyle`，使普通 italic、默认 oblique 与作者角度在选中同一实际字体面时仍有独立身份，回退继续沿用来源样式。原生 `ital`／`slnt` 在塑形前配置：italic 优先 `ital=1`，oblique 优先反号 slnt，按实际轴范围夹紧；没有斜体面或轴的直立字体才创建带剪切矩阵的绘制实例，原字体继续供 Core Text 塑形。真实斜体、已有倾斜矩阵或已配置原生轴不再合成。字形边界、装饰墨迹与栅格化共用同一原生绘制实例，合成不改变 advance、连字和插入点。回退原生轴仅在确需变更时按原 UTF-16 范围写回，再整行塑形一次；静态字体不增加塑形次数，既有实例命中时跳过轴字典复制。没有新增字体字节副本或并列缓存。栅格尺寸包含边缘和子像素扩展，分配前检查单维 8192px、实际缓冲 16MiB，尺寸和原点使用受检算术。macOS 详情已接入合法角度，±90° 可进入原生面与轴匹配；纯合成仍受栅格尺寸限制。依据 [CTFontCreatePathForGlyph](https://developer.apple.com/documentation/coretext/ctfontcreatepathforglyph(_:_:_:))、[CTFontDrawGlyphs](https://developer.apple.com/documentation/coretext/ctfontdrawglyphs(_:_:_:_:_:)) 和 [CTFontGetBoundingRectsForGlyphs](https://developer.apple.com/documentation/coretext/ctfontgetboundingrectsforglyphs(_:_:_:_:_:)) 的字体矩阵及用户空间位置语义。新增无窗口回归覆盖正负角度、普通斜体、塑形不变、实际回退、墨迹/像素、缓存隔离、真实斜体不重复倾斜和过大栅格拒绝；后端库交叉类型检查已通过，原生回归、变量轴及实际界面仍待验收。请求端连续 wght 匹配与缺少粗体面的源码处理由上述后续补丁补齐，仍无原生验证。

- Apple 原有断连字测试引用未保存的上游字体，现改为本包内 `tests/fonts/lilex/Lilex-Regular.ttf`，新样式回归复用同一文件；不进入应用资源或发布安装包。字体及 `OFL.txt` 均原样来自固定 Zed 修订 [f816e1f9263e3a8ef63c4baaa02e1441ce1c6038](https://github.com/zed-industries/zed/tree/f816e1f9263e3a8ef63c4baaa02e1441ce1c6038/assets/fonts/lilex)，保留 SIL OFL 1.1 与版权。字体 SHA-256 为 `8a467ad5a1941dc7c9cebaee34e3ed9364a73b48393034e363e4b528bd711d04`，许可证 SHA-256 为 `a356708a399e3f54b937ec7d9b722c26dabba21365b60f95c56eb7ebca0dee40`。

- Apple 字体索引和家族内去重使用完整 CTFont 的 CFEqual/CFHash，保留实际字号、变换、特性及可变轴实例；字号不归一化，避免重算自动光学字号轴。Core Text 回退结果直接交 font-kit 的原生句柄入口，不再经 CGFont 重建；回退的实际字号和字体度量接入原生绘制，既有断连字的一 ULP 字号微调不扩大行高。零字形运行段不创建绘制段或字体实例，复用原索引且不增加字体字节副本。现有后端测试补充同名不同字号/矩阵、等价实例复用和字号传递；后端库交叉类型检查已通过，原生测试未运行。作者角度与普通斜体的匹配、回退和绘制由上述 Apple 样式补丁接入，实际显示待验收。原生实例语义依据 [Apple Core Text 概览](https://developer.apple.com/library/archive/documentation/StringsTextFonts/Conceptual/CoreText_Programming/Overview/Overview.html) 与 [CTFontCreateWithGraphicsFont](https://developer.apple.com/documentation/coretext/ctfontcreatewithgraphicsfont%28_%3A_%3A_%3A_%3A%29)，绑定与句柄入口已对照固定版本源码。

- Linux 字体实例保留请求样式，使主字体缺少 italic/oblique 面时，Cosmic 仍按该样式匹配及标记合成斜体；输出消费已有 `FAKE_ITALIC` 标记，将合成状态纳入既有实例身份和 FontId。轮廓栅格化、装饰墨迹求交与按轮廓点确定的 GDEF 连字插入点使用同一 14° 剪切，避免真实斜体再被倾斜；常规坐标插入点与水平 advance 不变。系数仅随字体实例创建，字体字节和 Cosmic 所有者继续共享，不增加逐字形塑形或独立缓存。合成时优先使用可变换的轮廓，纯位图及彩色位图仍受 Swash 原有变换能力限制。字体属性回归继续检查真实所选字体面，而不把样式上下文的 FontId 当作物理字体面身份。原生 `ital`／`slnt` 轴现由下述共享实例链处理；作者角度现由下述 Linux 原生链传递，实际显示待人工验收。

- Linux 原生 `ital`／`slnt` 轴随请求样式进入 Cosmic 既有字体实例，匹配、回退、塑形、度量、GDEF、墨迹和栅格化共享完整归一化坐标。真实可达样式参与候选匹配；原生轴已产生倾斜时不叠加合成倾斜。输出身份及字形缓存包含请求样式，静态字体和等价坐标继续共享原 Arc，不新增并列缓存。默认 oblique 使用 OpenType `slnt=-14`，italic 优先 `ital=1`，缺轴仍使用既有回退／合成路径。作者角度贯穿主字体匹配、回退、属性、字形和既有缓存身份；匹配优先宽度与倾斜方向/角度，避免不带角度的 fontdb 查询覆盖已排序结果。保持 GPUI 的 italic/oblique 共享匹配尺度，italic 强度 1 对应 11°；OpenType slnt 与 CSS 角度取反。原生范围外请求共享夹紧后的端点实例，合成轮廓、GDEF 和墨迹复用同一实际角度。大剪切栅格化前检查轮廓边界，限制单维 8192 像素、RGBA 16 MiB，超限明确报错；没有增加字体缓存层。±90° 可进入原生匹配，纯合成端点不栅格化；纯位图变换仍受 Swash 能力限制；Windows 与 Apple 角度接入见各自补丁，Apple 尚无原生验证。

- Linux 可变字重以字体面、特性/回退配置和实际字重共同标识已有字体实例。候选匹配使用 `wght` 轴可达的字重，实例交给 Cosmic 现有字体所有者共享；静态字体沿用已选字体面的字重。塑形属性、回退字形输出、度量、advance、墨迹轮廓和 Swash 栅格化使用同一实例的归一化坐标。Swash 坐标只在实例创建时转换一次；空坐标也显式替换缩放器状态，避免从先前可变字体继承坐标。输出回退沿用既有实例映射，按字体面及塑形实际字重查找，不逐字形扫描全部已载入字体。字体新增后的重新解析同步更新可变实例的用户回退链。没有增加并列字体缓存或再次塑形；仍受 Cosmic `Weight(u16)` 的字重精度约束，作者斜体角度由上述 Linux 样式实例链处理。

- `Interactivity::capture_scroll` 与 `InteractiveElement::capture_scroll` 允许宿主在原生容器处理前接管滚动，复用现有监听分发、命中与遮挡裁剪；未调用的元素保持原有行为。浮层标签栏用它保留主轴选择、28px 行增量、1.2 倍速度和边界事件归属，不增加滚动状态、全局监听或几何缓存。原生交互待人工验收。

- `Platform::on_reduce_motion_change` 与 `App::on_reduce_motion_change` 交付原生偏好变更通知，由 Base 读取并沿用应用覆盖策略；通知本身不改写应用选择的状态。macOS 复用 Apple 通知观察者，在 NSWorkspace 专属通知中心注册辅助显示选项变更，注册与前台任务随平台释放。Windows 使用既有隐藏顶层窗口接收 `WM_SETTINGCHANGE`，同一轮消息合并到既有前台执行器，避免同步系统广播重入 GPUI；没有新增窗口、线程或轮询。App 回调在完整 update 中刷新效果。其余后端保留默认无操作，Linux 继续使用 Base 的 Portal 订阅。macOS/Windows 的实际编译和系统交互待原生环境验收。原生通知依据 [Apple NSWorkspace 文档](https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayoptionsdidchangenotification) 与 [Windows WM_SETTINGCHANGE 文档](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-settingchange)。

- Base 的 `DecorationMetric::calculated` 接受宿主已构建的有界长度表达式求值器，不把 CSS 解析引入 Kit。上划线、下划线、删除线共享同一解析对象，字体/视口度量沿用现有窗口入口，最终 NaN 按 CSS 规则先归零，再使用已有的设备像素下限和长度上限。表达式由受控文档持有并计入预算，不新增全局缓存、正文副本或塑形调用；原生呈现仍待人工验收。

- `Anchored::keep_position` 保留宿主已经计算的位置、分数坐标和有意的越界，不再自动翻转锚点或二次贴边。
  应用右键菜单按原来的估算槽位、边距、重叠与鼠标进入位置定位；工具栏菜单使用触发器的局部锚点，
  跟随实际布局。只有显式调用的菜单启用该策略，原有定位策略保持默认行为。尚未编译或验证。
- `HoverListenerMode::PointerPresence` 允许宿主显式跟踪按下或拖动期间的指针存在状态，键盘输入不使其失效。
  定制菜单用它保持悬停与按下反馈、主/子菜单离开计时的一致性；默认模式和仅忽略键盘的模式保留原有行为。
  此扩展没有独立状态表或计时器，复用现有命中与元素悬停状态；尚未编译或验证。

- `StyledText::with_additional_spacing` 接收共享的有序 `ClusterSpacing`，每个原文字簇
  分别记录逻辑前侧扩展与后侧间距，同一字簇只接受一个合并后的条目。针对已经断行的
  单行关闭再次换行、截断和省略。保留原文与字节索引，通过写时复制调整行度量中的
  字形位置、前进量和总宽度，不修改原生 shaping 缓存；绘制、下划线、背景、命中与
  选区使用同一份几何。前侧扩展计入单元宽度并移动其字形与连字内部停靠点，RTL
  交换逻辑前后侧对应的物理方向；后侧负值用于收起可折叠行尾空格，
  收缩不产生负的字形前进量。保留布局缓存包含间距键，共享实例直接比较指针。
  原生下划线、上划线、删除线与文字底色也用同一前进单元界定起止，字形墨迹位置
  仅用于栅格与轮廓避让，避免前侧间距在样式边界漏画。底色路径不再累计无用的
  墨迹横坐标，没有底色的行直接跳过底色遍历、字体包围盒查询及绘制层创建。
  `LineWrapper::wrap_line_with_space_policy` 可推迟行尾 ASCII 空格触发的溢出，
  仍返回原文边界；调用者按同一规则处理最终度量，原 `wrap_line` 保留原有策略。
  结构化文本可另行提供 `TextSpacePolicy`：统一策略或原布局 UTF-8 坐标中的
  有序区间，与 no-wrap 范围独立。`pre-wrap` 行尾空白推迟溢出判断，
  `break-spaces` 在每个保留空白或其他 Zs 分隔符后提供折行机会；两者仍遵守
  字素、原生字簇和 no-wrap 边界。nowrap 组仅在实际末尾的可折叠/悬挂
  空白可被排除时扣除该后缀用于溢出判断，不减少组内源文本或实际前进量。
  续行借用同一策略区间，不另存逐字符表；未显式配置的调用保留原有行为。
  这些空白策略补丁尚未编译或验证；软行制表位范围测宽见下文。
  以上改动尚未编译或验证。
- `TextStyle::tab_size` / `Styled::tab_size` 为结构化文本提供继承的制表位配置，
  默认不设置，不改变原生输入或普通标签。`ShapedLine::with_tab_stops` 在完整硬行的原生
  双向排序之后，按段落起始边及实际空格宽计算制表位，最小前进量由调用者提供，
  零宽制表位收起 Tab。只为真实 Tab 保存临时位置与稀疏字簇间距；复杂双向文字按视觉
  次序处理，而非假定 Tab 的源码顺序等于绘制顺序。复用既有 `add_spacing` 更新原生
  字形、前进量、内部光标及总宽，不替换源文本，也不污染共享塑形缓存；累计位移改用
  f64，避免长行的逐次 f32 累加放大误差。原生未提供独立 Tab 单元时明确返回不可用。
  该入口只调整保留行的制表位；断行使用下文的范围测宽。尚未编译或验证。
- 场景 `Quad`、`Shadow`、`Underline` 和 `PolychromeSprite` 追加已有的
  `TransformationMatrix`；Metal、DirectX 和 WGPU 顶点使用变换后的坐标及窗口裁剪，
  片段通过原坐标插值计算圆角、渐变、边框、下划线和阴影，避免非等比缩放改变形状计算。
  Apple 生成着色器头文件所用的源副本同步维护布局；WGPU 的 storage/WebGL 两种传输
  同步记录步长，WebGL 四边形使用 word cursor 处理跨 texel 的新记录边界。
  场景按变换后的包围盒安排裁剪和绘制次序，阴影覆盖包含模糊外缘。
  窗口保存逻辑坐标变换和逆矩阵，物理绘制只按 DPI 换算平移量；内容裁剪、命中包围盒
  与场景使用窗口坐标，鼠标位置和文本输入点查询反向映射，滚轮仍保留逻辑滚动量。
  字形、路径、SVG、图片和表面共用作用域，变换期间关闭子像素字形，避免颜色边缘。
  视图缓存键包含矩阵；按键、动作与输入处理器保留注册时作用域，IME 候选框、选区、
  输入边界和无障碍节点输出窗口坐标；零缩放没有命中目标，但仍传递释放/取消通知。
  `scaled` 保持布局分配、以左中点缩放，供搜索与标签关键帧使用；不生成位图快照。
  Deferred 弹出层保留变换后的锚点和自身尺寸，显式裁剪同时覆盖预绘制和绘制；Base
  popup、Kit 提示框、菜单和按压区域统一使用窗口坐标。以上改动尚未编译或验证。
- `Window::set_visible` 隐藏或重显单个窗口，保留句柄及视图状态，显示不请求焦点；
  不支持的平台明确返回错误。初始 `show=false` 不再被无条件的 X11 map 覆盖。
- `Window::set_pointer_passthrough` 在保留 drawable 的同时切换原生鼠标穿透，
  不通过隐藏窗口或只返回 `HTTRANSPARENT` 冒充跨进程穿透。X11 使用 SHAPE 1.1
  的 input region，能力查询按窗口缓存，恢复时使用默认区域以适应后续尺寸变化；
  Wayland 提交空/default input region；macOS 保存并恢复 NSWindow 原有鼠标忽略值；
  Windows 临时添加 layered/transparent 样式，仅在首次添加 layered 时初始化 alpha，
  恢复仅修改本接口拥有的两个样式位，初始化失败尝试回滚并报告错误。
  Win32 失败使用锁定的 windows 0.62 `Error::from_thread` 读取线程错误，保留错误上下文。
  原生窗口准备/隐藏时保持穿透，首帧屏障完成后才恢复输入并激活，不支持时明确失败。
  Windows 实现依据 [layered window hit testing](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features)
  和 [DirectComposition target window](https://learn.microsoft.com/en-us/windows/win32/api/dcomp/nf-dcomp-idcompositiondesktopdevice-createtargetforhwnd)。
- `Window::request_frame_submission` 强制请求新场景，等待该场景或更新场景真正提交。
  Windows、Metal、X11/Wayland wgpu 都报告真实结果；无 drawable、设备恢复、
  绘制失败、旧场景再次呈现均不完成屏障。此接口保证提交，不保证合成器扫描输出。
  接收端销毁即取消等待；单窗口最多保留 64 个有效等待。
- Wayland 隐藏解除 buffer 绑定并停止该窗口帧调度；重显按协议恢复 layer-shell/
  xdg 参数，在新的 configure/ack 后恢复提交。旧 frame callback 通过对象 ID 过滤。
  首次尚未映射的隐藏窗口复用已经完成的初始 configure。
- 复用上游已有的 `WindowKind::LayerShell`，不维护第二套 Wayland 绘制宿主。
  动态键盘模式用于准备阶段和可交互阶段切换；OnDemand 在协议版本低于 4 时返回错误。
  公共窗口和平台 trait 的该接口限定为 Linux + Wayland，避免其他平台引用不存在的协议类型。
  exclusive zone/edge 修改也会更新重显时恢复的状态。
- `Window::activate_window_with_token` 将 portal 或二次启动携带的令牌直接交给
  Wayland `xdg_activation_v1`；不重新生成令牌替换已经授权的令牌。
- macOS 原生窗口激活在既有前台执行器及关闭检查内，先对已最小化窗口调用
  `deminiaturize`，再执行 `makeKeyAndOrderFront`；不新增线程或窗口状态。
  应用层通过既有激活策略 API 切换后台与设置窗口模式。
  原生恢复依据 [Apple NSWindow 文档](https://developer.apple.com/documentation/appkit/nswindow/deminiaturize(_:))；
  macOS 原生编译与交互尚待验收。
- macOS 保存面板在请求时取得父窗口，沿用前台执行器显示系统原生 sheet；
  父窗口在显示前隐藏则取消，未指定父窗口的调用仍保留原有独立面板行为。
  仅用户取消返回空结果，异常响应和成功却无本地路径均返回错误；完成时先
  `orderOut` 再交付结果，避免后续反馈被尚未退场的面板遮挡。
  Linux 保存 Portal 同样只将 `Cancelled` 作为取消，其他响应、空路径及无效/
  非本地 URI 均报错。没有新增文件选择器依赖或窗口状态。
  依据 [Apple NSSavePanel](https://developer.apple.com/documentation/appkit/nssavepanel/beginsheetmodal(for:completionhandler:))
  与 [XDG Request 响应](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Request.html)。
- `KeyDownEvent` / `KeyUpEvent` 增加可选物理键元数据：X11/Wayland 的 XKB
  keycode、Windows 扫描码（含 E0 前缀）、macOS virtual keycode。录制器据此识别
  布局无关的键位；合成输入与 IME 提交不伪造物理键。现有逻辑键绑定匹配不变，
  Base/Component 的事件构造同步填写 `None`。
- `RenderImage::from_shared_bgra` 包装不可变、紧密排列的直通 alpha BGRA8
  像素所有者，校验尺寸与长度，保留所有者直至图片释放。不再为满足 GPUI
  `image::Frame` 的 Vec 要求复制整张图片，也不经 PNG 再编码、再解码。
  已有 Frame/动图入口不变；新入口只表示单帧，GPU 上传和驱动分配仍然存在。
- `RenderImage::from_shared_premultiplied_bgra` 接收图片服务最终产生的预乘
  BGRA8；`PolychromeSprite` 将原填充字改为 `image_flags`，bit 0 标记此输入，
  不改变结构大小、偏移或 WebGL 按字读取的布局。Metal 的绑定来源镜像同步此字段。
  Metal、D3D 与 WGPU 对此入口将采样坐标限制在当前图块的像素中心范围，放大时
  不采入邻接 atlas 图块；在线性采样后解除预乘，再使用各自已有的混合状态处理
  透明度、圆角覆盖与目标表面 Alpha 模式，避免二次乘 Alpha。直通共享入口、
  既有 Frame 和彩色字形的标记为零，保持原行为；不增加纹理采样次数或 CPU 副本。
- `StatefulInteractiveElement::aria_hidden` 将隐藏状态传递给 AccessKit，
  用于保留布局且渐隐的快捷提示；仅控制无障碍树，不改变绘制或输入语义。
- `aria_busy` 与 `aria_invalid` 通过原有元素语义投影写入 AccessKit；未设置时保持默认状态。
  标签列表与编辑字段分别报告读取、提交状态，输入校验失败落在真实编辑节点，
  不借另一个可聚焦控件代替。无独立语义状态表或订阅；尚未编译或验证。
- `ListState::measured_content_height` 只读取已有稳定布局摘要，不触发额外测量或复制列表。
  模型下拉对不足八项的列表一次测量全部项，按真实换行高度收缩；更多项保留可见项布局。
- `Styled::grid_cols_auto` 将内容列映射为 Taffy 的 `minmax(auto, auto)`，
  供统计表头和全部数据行共享列测量并填充余宽；原有等宽、min-content、max-content 轨道不变。
  `GridTemplate` 的可选固定首轨道及 `grid_cols_auto_with_leading` 用于详情数据表格的
  28px 行号列，其余列共用内容测量；`grid_cols_with_leading` 为卡片提供固定行号与
  等宽内容列，同一张表的各行共用轨道；只有行号时不生成零次 repeat。
- `LineFragment::shaped_text` 共享已有原生 glyph 布局，以单向游标提供字形宽度，
  保留现有 Unicode 断行和字素边界，不生成逐字宽度数组或逐字重新 shaping。
  双向文字的视觉字形顺序与源码不同，仅该路径生成共享的字形簇宽度索引；
  换行后的片段复用索引，不逐字符重新 shaping 或反复扫描整个布局。
  `LineFragment::NoWrap` 保持受控富文档中的不可换行范围完整，源文本和复制内容不变。
- `ShapedGlyph::advance` 保留各平台塑形器的真实前进宽度，独立于字形墨迹偏移。
  DirectWrite 读取原生 advances，Core Text 优先借用 advances 指针、缺失时读取单项，
  Cosmic 使用原生布局宽度。Linux 输出按视觉位置稳定排列，并保留字体的 x/y 字形偏移；
  macOS 在源索引首次回退时建立一次临时 UTF-16/UTF-8 映射，避免重复扫描前缀。
  断行和 Base 范围测量累加真实 advances，不再用相邻墨迹位置差推算宽度。
  复用字形的紧急断行同时要求字素和塑形簇边界，避免拆开连字后留下无字形的后半段。
  `LineLayout::split_at` 按源区间筛选并重排片段内的前进位置，不对视觉顺序使用二分切片；
  `ShapedLineCursor` 的双向路径只建立一次源索引，各片段原地恢复视觉顺序，
  不反复克隆剩余整行。字体查询也按源索引匹配，保留同一簇的首字体。
  这些改动保持已有塑形结果，仍不实现跨连字切口的局部上下文塑形；
  当前尚未编译或验证。
- `text_system/line_layout/bidi.rs` 为实际溢出的双向原生文字先按源序选择软断行，
  切口同时满足字素与塑形簇边界，再以整段已解析的级别逐行调用现有 `unicode-bidi`
  的 [UAX #9 L1/L2](https://www.unicode.org/reports/tr9/#L1) 重排；不把每一行重新当作
  独立段落解析。级别与类别仅复制当前行，临时缓冲跨行复用，避免按每行复制整段。
  原生 glyph、advance、偏移与簇内 caret 移到新行的物理位置，同字体 run 可跨行共用
  字形缓冲，整个字素/塑形组使用同一位移以保留附标与墨迹偏移。强制 RTL 的纯 LTR
  内容及自动方向中的不可见 ALM/RLM 也进入该路径，不只检查可见字形的方向。
  必要的行顺序字形布局进入原有宽度/方向/字体缓存，未增加常驻文本副本；
  普通 LTR 与无需换行路径保持原实现。`source_row_ranges` 提供逻辑行区间，
  `TextLayout::wrapped_text` 不再把 RTL 行首绘制字形误当作源码切口。
  按 [CSS Text 3 的词内软换行规则](https://www.w3.org/TR/css-text-3/#line-breaking)
  保留整词的连接形，不把每一行作为独立词重新塑形；跨现有连字切口的上下文塑形、
  行内隔离及完整双向排版仍未完成。应用的能力限制继续保留，尚未编译或验证。
- `ShapedGlyph::is_rtl` 保存原生解析后的方向，Core Text 读取 run status、DirectWrite
  读取 bidi level、Cosmic 读取 layout level。基础 `index_for_x`、`closest_index_for_x`
  和 `x_for_index` 以视觉字形簇的前进区间计算，RTL 的逻辑起止使用相反的物理边缘，
  不按 UTF-8 源偏移的递增顺序猜测视觉位置，也不把墨迹偏移当作插入点。
  不增加常驻几何缓存。未换行行的越界命中保留真实物理边缘对应的源偏移；
  行坐标使用物理行原点，避免减去 RTL 的逻辑首光标后产生负坐标。
  固定字宽布局同时更新基字/附标前进宽度及整行宽度；Base 的 Tab 展开同步更新
  Tab 前进宽度，普通文字行的范围宽度使用 advances。游标内部累计宽度与公开 caret
  坐标分离。默认行内 caret 保留后继簇首边缘，默认软断行位置保留前驱簇末边缘；
  完整双向换行仍未完成，尚未编译或验证。
- `LineLayout::clusters` 暴露原生簇的源区间、advance 单元和首字形位置。
  普通源序无索引分配；乱序布局只建立一次临时源索引，不逐字扫描下一个源位置。
  `visual_row_ranges` 不分配内存，单次遍历给出已有断行的物理 advance 区间；
  断行候选、绘制行原点、装饰线末端和命中共享这些度量，保留行首字形的墨迹偏移。
  跨行继续的背景和装饰线从下一行的实际对齐原点开始。
  `CaretAffinity` 与显式位置入口提供源边界的前驱/后继两侧，使用字形所属行定位，
  不使用递增源偏移判断视觉行。`TextLayout` 保留对齐方式并纳入复用条件，绘制、
  对齐后的命中和光标使用同一值；最近插入点入口供选择与复制复用。
  `TextHit` 同时返回 UTF-8 偏移和实际命中侧，整行、软行和元素坐标入口均保留
  越界夹取结果的 affinity，不迫使上层从一个源偏移猜测物理插入点。
  `StyledText::without_truncation` 为持有源区间的文档关闭继承的省略号/原生行数截断，
  保留 wrapping 和源字节，由文档容器裁剪；默认控件的截断入口不变。
  TextLayout 复用条件包括此策略、实际换行宽度及行数上限，不再用已换行尺寸回答
  无约束或 nowrap 测量；策略变化也不会继续复用旧的截断源文本。
  Base 编辑输入已按光标保留命中侧并接入鼠标/触摸、垂直移动及 IME；范围绘制与
  菜单命中共用原生字形簇区域，具体范围见 KIT-PATCHES.md。视觉左右导航、
  换行重新塑形和行内隔离仍待实现，尚未编译或验证。
- `LineLayout::interior_carets` 只为原生塑形簇内部的字素插入点保留源偏移、所属簇和
  物理坐标，按源顺序排列；普通单字素簇不增加条目。字素扫描借用完整源前文，但
  不扫描超过当前簇的后文；连续源簇保留 Unicode 游标上下文，避免重复计算 RI 序列。
  macOS 在已有 CTLine 上查询位置，Windows 查询已有 DirectWrite text layout，
  UTF-8/UTF-16 索引均按源顺序推进，不建立新的逐字符索引。
  Linux 使用 Cosmic 重导出的 Skrifa，借用现有字体字节及正确 TTC face，读取
  GDEF 格式 1/2/3 与浮点变体修正，使用塑形器的相同变体坐标；没有新依赖或字体缓存。
  格式 2 通过同版本 Skrifa 的限定接口取得原始轮廓点，复用复合字形与变体处理，
  保留小数精度，不从绘图路径重建点编号。每字形只展开一次，临时保存需要的点位置，
  不复制完整点阵；来源、许可证与接口见 `skrifa/PATCHES.md`。
  未提示布局不应用栅格设备修正。组件与字素数不一致的精确对应仍未实现，
  保留比例位置；缺少 GDEF 的字体也使用比例位置，异常表明确记录。
  命中与位置查询共用这些数据，`cluster_cells` 提供连字内部的选区单元；不重塑形
  或把组合序列按字符均分。切片、段落拼接、间距、固定字宽和 Tab 展开维护相同坐标，
  固定字宽只为有内部插入点的簇临时保留旧单元。数据随原有布局回收，不增加独立缓存。
  各平台实际行为及组件对应等缺口尚待完成或集中核实，尚未编译或验证。
- `TextSystem::font_generation` 为保留布局提供字体集代次；`TextLayout` 的尺寸缓存
  与 Base 的 InlineFlow 一并检查该代次，注册字体后重新测量，不继续复用旧的字体度量。
  不增加第二套字体缓存；尚未编译或验证。
- `TextDecorationPattern` 将实线、双线、点线、虚线、波浪传入现有下划线与删除线管线；
  保留 `UnderlineStyle::wavy` 的旧优先语义，新字段反序列化默认实线，既有 solid
  样式方法同时重置新字段。颜色解析、共享标记、字体度量与淡出保留图案；本地完整
  struct 字面量及已有测试的几何字段对比同步更新，没有新增 UI 测试。
  `Underline` 的原填充字改为 `pattern_period: f32`，旧四字节波浪标记改为图案/区间
  标志，大小、偏移与 WebGL 的 22 字读取步长不变。Metal 绑定来源镜像、WGSL 的
  storage/WebGL 输入、HLSL 和 Metal 着色器一并接入；非实线仍使用原图元，不按
  点或虚线段建立 CPU 列表。周期按图元计算，裁剪/排除保持原 bounds 与周期，
  删除线的双线/波浪高度围绕原单线中心扩展。窄虚线间隙按像素区间积分，避免
  最近边缘距离低估双侧覆盖。
  Windows 区间参数参考 [Chromium styled stroke](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/platform/graphics/styled_stroke_data.cc)，
  macOS/Linux 参考 [WebKit text lines](https://github.com/WebKit/WebKit/blob/main/Source/WebCore/platform/graphics/GraphicsContext.cpp)。
  未增加 shaping、纹理或缓存；跨文字片段的相位、浏览器端点/位置仍待对照。
  尚未编译或验证。
- 下划线增加默认关闭的 `UnderlineStyle::skip_ink`，实际 `ShapedLine` 和 `WrappedLine`
  原生绘制可显式启用 CSS auto 墨迹避让；公开自定义回调仍完全管理自己的绘制。
  `Window::paint_glyph_with_raster_bounds` 返回该次 atlas 绘制已有的设备坐标边界，
  不重复查边界或栅格化；文字绘制先用它排除空白/未触及条带的字形。
  `PlatformTextSystem::intersect_glyph_ink` 将当前原生轮廓直接送入临时 `GlyphInk`：
  DirectWrite 用几何 sink，Apple 用已选 Core Text 字体并只读取所需 units-per-em，
  Cosmic 借用现有字体字节、TTC 索引与归一化变体坐标，通过现有 skrifa 输出轮廓。
  公共累加器用 f64 曲线求根/横向极值计算条带内投影，同一字形的上下条带合并为单次遍历，至多四条带使用内联存储；
  不建立第二份 path、CPU 位图、字体或字形缓存。Windows 同步 COM sink 持有该次
  小型累加器；仅位图字形无轮廓交集，真正的读取失败不替换成宽泛包围盒或静默成功。
  Noop 后端与其空栅格一致地返回无墨迹。
  双线分条带裁剪原图元，保留其 bounds/period；临时 x 间隔在换行和样式结束时清空。
  借用原文字串并复用锁文件已有 unicode-script 0.5.9 的 Script_Extensions 判定 CJK，
  不在字形中复制文本/脚本属性；没有源文字的裸布局不自行猜测。默认关闭的输入法、
  诊断线和既有完整字面量保留原策略；现有内部回调测试只作接口适配，没有新增测试。
  条带内缩/留白参考 [Chromium ink clipping](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/paint/text_painter.cc)。
  原生轮廓与栅格细节、边距和平台实际效果仍待对照。尚未编译或验证。
- `TextStyle`、`HighlightStyle`、`TextRun` 与 `DecorationRun` 增加原生 `overline`，
  转换、哈希、合并、切片和自定义下划线回调路径均保留它；旧回调只接管下划线。
  `Window::overline_bounds`/`paint_overline` 复用已有装饰图元和条带裁剪，
  单线、双线及波浪从 text-top 向上扩展，文字绘制层包含上方溢出的墨迹，
  不改变布局和命中。上划线复用已有 skip-ink 与脚本规则；同时存在的上下
  装饰在同一 glyph 查询中合并条带，重合层复用查询条带但独立合成；
  四条带保持内联存储，更多层按需扩展，不增加轮廓遍历或栅格化次数。
  未新增 GPU 字段、纹理或持久缓存。同类祖先叠层通过下述共享来源链提交；
  各平台字体位置对照与跨片段相位仍待处理，尚未编译或验证。
- `UnderlineStyle`（含上划线）和 `StrikethroughStyle` 增加默认空的
  `baseline_offset`，表示相对当前塑形基线的逻辑像素偏移；None 保持既有原生定位。
  富文本宿主以装饰来源的字体和基线计算该值，防止后代字号、上下标移动祖先装饰，
  遵循 [CSS 装饰盒定位](https://www.w3.org/TR/css-text-decor-3/)。
  该字段参与既有样式相等性、哈希和运行转换，不新增 GPU 字段或缓存。
  直接 Window 绘制接口仍接收已定位的 origin，不重复应用相对偏移。
  `Window::strikethrough_bounds` 与实际绘制共用吸附和形状扩展几何；文字层边界
  同时纳入三种装饰的偏移和粗细溢出，包含首末折行，不改动布局尺寸、选区或命中。
  skip-ink 使用更新后的线条位置，仍复用同一次字形墨迹查询。
  既有控件及回调字面量只适配新增默认字段，未新增 UI 测试。尚未编译或验证。
- `TextRun`/`DecorationRun` 可携带 `DecorationLayers`，存在时替代三个传统样式槽。
  不可变 `DecorationLayer` 共享父节点；稳定身份复用来源的 Arc 分配，既区分外观
  相同的独立装饰盒，也避免每帧新解析的节点使相同布局失效。相等性比较身份、
  解析样式与父链；合并、切片和保留布局完整传递该信息。片段基线位移独立保存。
  绘制时用临时有序线段替代固定三槽，按线型与祖先深度线性合并状态，保留父线
  跨子样式片段的连续范围。结构化文字收集本次绘制的已完成线段和字形图元，
  依次提交下划线、上划线、字形和删除线；同类装饰按祖先到后代合成，并用独立
  绘制层避免 Quad 与 Underline 图元批次重排颜色。父线不因子线收尾而拆段，
  避免因此重启虚线/波浪的相位。普通原生控件保留即时绘制路径。
  `prepare_glyph`/`prepare_emoji` 复用既有图集、栅格边界和绘制状态；图元只提交
  一次，字形不再次塑形、栅格化或查询图集。`PreparedGlyph` 只容纳三种字形精灵，
  不为每个字形预留通用 Quad/Path 图元空间。暂存的图元、线段与墨迹间隔只属于
  本次调用，结束即释放，不构成跨帧缓存。多层 skip-ink 仍共用一次轮廓查询。
  尚未编译或验证；平台对照与跨独立布局片段的图案相位仍待处理。
- `TextSystem::underline_thickness` 暴露已有字体度量中的推荐下划线粗细，供原生
  富文本的 `from-font` 使用；与 ascent、x-height 等入口共用字体度量缓存，
  不增加字形塑形、平台接口或独立缓存。尚未编译或验证。
- `TextSystem::normal_line_height` 复用同一度量缓存，以首字体的 ascent 减去带符号的 descent
  和 line gap 计算正常行高，供富文本根单位 `rlh` 使用，不依据正文回退字形或 Kit
  的显式行高推测。Cosmic 后端的 `serif/sans-serif/monospace` 先解析为已加载字体族；
  Linux/FreeBSD 启动时通过已有 font-kit/fontconfig 查询系统选择，以路径和 TTC index
  对应已有数据库记录，替换 Cosmic 的硬编码默认名称，不重新读字体文件或新增缓存。
  选择失败或所选字体不在数据库时记录警告并保留该通用族原默认值；无系统字体的
  构造入口不查询系统。根字体基准参照 [WebKit 平台默认值](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WTF/Scripts/Preferences/UnifiedWebPreferences.yaml)、
  [GTK 默认设置](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebKit/UIProcess/API/glib/WebKitSettings.cpp)
  和 [Blink 默认值](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/third_party/blink/common/web_preferences/web_preferences.cc)。
  浏览器对 normal 行高的取整、系统字体对应及跨平台呈现仍待集中核实，尚未编译或验证。
- `TextStyle::text_direction`、`Styled::text_direction` 与显式方向的 shape/layout
  入口将段落方向传至 `PlatformTextSystem`；旧入口继续采用 Auto。布局缓存键区分
  方向，换行缓存同时区分行数上限；保留的 TextLayout 和 Base InlineFlow 也按方向失效。
  修正多行 shaping 第三行起的源偏移推进，保持源行区间与输入一致。
  Windows 使用 DirectWrite 阅读方向，先在物理左侧测量，再以含末尾空白的实际宽度
  设置有界的 leading 布局，避免 RTL 对无穷宽盒对齐产生无效坐标；禁用原生自动换行，
  在全部字体样式应用后取行高与基线。macOS 使用 Core Text 的段落基方向属性；
  Linux 使用 `cosmic-text` 0.19.0 的限定补丁，见 `cosmic-text/PATCHES.md`。
  这些路径不插入方向控制字符，不改变源字节及选区索引，也不叠加常驻布局缓存。
  此项只补齐段落参数链，不代表跨连字切口的上下文塑形、行内 isolate 或完整 bidi 选区已完成；
  尚未编译或验证。
- Core Text 的属性字符串一次性接收完整原文，再按原文的 UTF-16 范围设置字体，
  不再分别解码并追加每个字体片段，避免片段开头的 U+FEFF 被当作 UTF-8 BOM 消费。
  原文自身以 U+FEFF 开头时用 `CFStringCreateWithCharacters` 保留全部显式字符；
  临时 UTF-16 缓冲仅用于该分支，原生字符串复制后即释放，不增加布局缓存。
  普通文本只创建一次源 CFString；字体范围、原生字形索引和既有源索引转换器
  使用同一原文。依据 [Apple UTF-8 解码实现](https://github.com/apple-oss-distributions/CF/blob/main/CFStringEncodings.c)
  与 [显式字符构造入口](https://github.com/apple-oss-distributions/CF/blob/main/CFString.c)。
  尚未编译或验证，不代表软行上下文塑形已经完成。
- `WindowTextSystem::shape_line_with_sized_runs` 接受保持 UTF-8 字节范围的
  `SizedTextRun`，范围须完整覆盖原文，字号须为有限正值。它复用现有字体解析、
  布局缓存及字体代际；字号进入 `FontRun` 的缓存键，普通文字继承整行字号。
  `SizedTextRun::break_before` 为基线或行内盒上下文保留原生 run 边界，
  其他绘制差异沿用普通规则，背景颜色变化不单独打断字距与连字。
  DirectWrite、Core Text 和 Cosmic 在同一原生段落应用各 run 的字号；
  输出 `ShapedRun` 保留原始字号，绘制、栅格与连字插入点按该字号计算，
  不把区分装饰所用的临时字号扰动写入栅格缓存键。普通均匀字号不建立源字号索引。
  源切片、段落双向重排与 Tab 展开必须保留字号，合并时同时比较字体与字号。
  此入口不负责行内盒布局、基线位移、isolate 或断行后的跨片段视觉布局。
- `ShapedLine::with_default_font_size` 将保留片段的默认字号改为局部上下文字号，
  同时重写各 run 的继承关系，保留原有实际字号、字形、插入点与位置。
  它不缩放字形，不修改源缓存，也不把段落最大度量当作局部片段度量；
  调用方通过 `with_vertical_metrics` 提供片段的字体度量。
  升降部未改变时直接保留共享布局，避免不必要的写时复制。
- `LineFragment::with_no_wrap_ranges` 让完整原生文本借用源坐标下的有序、
  不重叠 nowrap 范围。范围内部禁止断行，原生簇与原文字素共同约束边缘断点，
  受保护空格不使用可折叠行尾策略。续片保留同一源坐标，不重置到局部偏移。
  范围游标随字符单向推进，不逐字符重扫范围；沿用已有字形及按需建立的双向
  字距索引，不分割测量片段、不替换为 ASCII 占位符、不增加索引或全局缓存。
  续行缩进为零时不额外测量空格宽度，完整原生断行可直接消费已有字距。
- `App` / `Platform::display_under_cursor` 查询系统光标所在显示器；Windows
  使用原生监视器身份和该屏 DPI，macOS 使用 AppKit 屏幕及其可用区域，
  不依赖当前 GPUI 窗口是否获得焦点；后端不提供查询时显式返回 `None`。
- Windows 的 DirectWrite renderer 使用回调的真实基线和原生 `ComputeGlyphOrigins`
  取得包含 bidi 方向、字距与偏移的字形位置，行宽使用包含末尾空白的原生度量。
  输出缓存按视觉顺序排列，字形簇同时接受递增、递减的 glyph 索引；无效或不完整的簇映射
  明确报错。跨 run 的源偏移发生回退时才建立一次 UTF-16 到 UTF-8 索引，避免反复扫描前缀。
  按[微软 API 的数组输出契约](https://learn.microsoft.com/en-us/windows/win32/api/dwrite_3/nf-dwrite_3-idwritefactory4-computeglyphorigins(dwrite_glyph_runconst_d2d1_point_2f_d2d1_point_2f))
  调用继承的 COM vtable 并提供 `glyphCount` 个结果槽位，绕过 windows 0.62.2 生成包装器
  仅提供单个 `Vector2` 输出空间的问题。位置数组仅在本次 shaping 存活，不增加常驻缓存。
  该补丁尚未编译或在 Windows 验收，不代表完整双向文字选区、断行或显式方向已完成。
- 通用文字绘制使用共享样式游标匹配视觉顺序中的源字节位置，颜色、背景、下划线与
  删除线不再只支持向前的样式迭代。只有发生反向样式切换时才建立前缀索引，八段以内
  保存在栈上；其后双向跳转均使用二分查询。下划线只合并连续的逻辑范围，比较实际
  解析后的颜色；换行沿用样式时同时检查源区间起止。背景和装饰线的末行终点共用实际
  对齐后的行宽，不依赖首个字形的偏移。这是绘制层修正，未完成双向选区与断行验收。
- X11 `PlatformDisplay` 查询根窗口当前尺寸，避免 RandR 改变布局后继续
  使用连接建立时的不可变 setup 尺寸。物理监视器与面板工作区由应用的
  X11 平台服务查询，再按对应 X Screen 的 GPUI 逻辑尺寸转换。

- `LineLayout::adjacent_visual_caret` 按命中侧寻找所属原生簇，再沿物理方向
  跨越簇内字素或相邻簇；进入相邻簇时跳过入口，避免把共享坐标视为同一
  源插入点。`visual_edge_caret` 提供有归属侧的实际外边缘。两者复用已有
  连字插入点与临时源索引，允许应用过滤非法字素或对象内部停靠点；不新增
  常驻字符表。此 API 本身不实现跨行、跨段或视口外塑形。
  `LineLayout::resolve_visual_caret` 将原生塑形省略的源前缀解析为绘制时已使用的
  首个逻辑簇起侧，不按横坐标寻找邻居。调用者仅在普通遍历失败后使用它，
  成功的普通移动不增加扫描；它只返回临时源命中，不改变复制文本或选区锚点，
  不创建字符表或缓存。非空但没有原生字形的行仍明确返回无可解析簇，
  不伪造其几何。这项补丁尚未编译或验证。

- `ShapedLine::shared_layout` 和 `StyledText::with_line_layout` 允许文档布局将既有
  字形传给最终文字元素，不再次解析字体或塑形。调用方负责源字节、字号、字体和
  字体代际的一致性；入口检查字节长度、单行源文本和字号。`None` 清除旧的预排版
  几何。保留布局还比较字形 `Arc` 身份，避免文本相同而上下文几何不同时误用缓存。
  当前颜色、背景和装饰仍由 `TextRun` 生成；选区、命中及绘制沿用同一个
  `TextLayout`，文档提供的源范围不被再次截断或换行。两端对齐的附加间距只在
  布局的写时复制副本上应用，不修改测量字形。不把上下文片段注入以独立文本为键的
  全局塑形缓存，也不增加第二套命中几何。
  `ShapedLineCursor::is_cluster_boundary` 识别原生簇切口，允许上层拒绝截断连字的复用；
  它本身不处理切口处的上下文塑形。`ShapedLine` 的装饰容器改为单段内联、多段按需
  分配，避免保留大量短硬行时每行预留 32 段存储。尚未编译或验证。

- `TextLayout::first_baseline` 从已测量首行读取相对元素顶边的实际绘制基线，
  与文字绘制共用 ascent/descent 和居中 leading；未测量或没有行时返回 `None`。
  不额外塑形、复制行布局或保存第二份度量，供 Kit 外侧列表标记定位使用。
  原生文字与 Kit 混合行通过 `Window::request_measured_layout_with_first_baseline`
  将尺寸和首行基线作为同一测量结果交给 Taffy，基线按相同 DPI 转换，并随已有
  布局结果缓存；普通测量入口仍返回无基线。没有额外基线闭包、Rc、缓存或整形。
  `Window::set_outside_list_marker` 给 Block 正文附加已测量标记指标，配合本地
  Taffy 0.15.1 的限定补丁处理较高标记的正文位移、正常流高度和无行框时的占高；
  空叶节点同样处理，保持零高度节点的原有折叠语义。
  `Window::layout_first_baseline` 从 Taffy 现有最终布局缓存读取相对盒顶的基线，
  按 DPI 换回逻辑像素；不克隆结果、触发布局或创建几何缓存。Kit 的表格宿主以此
  向列表标记导出原生盒基线，阻止首段后代文字抢占；无基线时保留 None。
  `Window::set_table_cell_alignment` / `set_table_grid` 为单正文块列 Flex 单元格
  与其 Grid 宿主设置限定表格元数据，原生布局统一处理对齐。通过原位样式更新
  保留正常祖先缓存失效，避免克隆轨道向量；单元格格子保持伸展，
  baseline 偏移进入完整布局缓存约束，在原生最终布局中移动流内正文，保持格子
  的 padding 和尺寸；尺寸缓存忽略该位移。不另存文字、几何或测量闭包。
  表格首行基线区分参与单元格、空内容、最低内容边缘及跨行格有效底部留白。
  `Styled::grid_rows_auto` 复用已有 Auto 模板表示，保留显式空行而不强制等高；
  表格宿主启用限定自动行高/跨行分配，列宽变化同步重算行及宿主高度。
  作者行/行组高度和百分比与 DPI 组合仍待补齐或对照；尚未编译或验证。
  Taffy 的来源、许可与维护边界见 `taffy/PATCHES.md`。

- `Window::set_collapsed_table_border_width` 在普通样式换算之后，给显式原生表格
  及其单元格赋予半个已吸附笔画的布局边框，返回完整逻辑笔画宽度。避免普通
  GPUI 边框转换把每个半宽分别向上吸附为一个物理像素；不新增 Taffy 节点、
  克隆样式或给普通 Grid 改变语义。绘制边框仍由应用宿主管理。
  `Window::set_collapsed_table_row_border_width` 复用同一路径，为完整、无合并
  单元格的表格处理仅有底边的边框：首行上边为零，内部行上下各半宽，表格
  只补足最外侧的底部半宽，左右不分配边框。统计表基于现有子盒按整行绘制
  一次边线，裁剪后仅保留本帧可见几何；不重复计算列宽或单元格尺寸。
  `Window::visit_unrounded_layout_children` 复用已有绝对原点缓存，访问现有直接
  子节点的未吸附布局盒，应用同一元素偏移和 DPI，不复制布局、重新测量或新增
  几何缓存。应用据此在背景之后、正文之前绘制统一细线的合并边框。
  `PathBuilder` 的填充临时索引改为 u32，允许单次 NonZero 填充合并大量可见
  边框矩形，避免旧 u16 输入上限；GPU 的非索引三角形格式、描边临时索引和
  原有公开 `build_path(u16)` 入口保持不变。仅由应用端生成、裁剪可见边线，
  不新增常驻路径缓存。尚未编译或验证。

- `Window::set_table_auto_columns` 对现有表格节点启用 Taffy 限定的内容列宽
  分配，要求无跨列的原生单元格，不插入布局节点。Grid 模板的首个定长
  轨道补齐 DPI 换算，定长宽度与普通长度同为逻辑像素。
- `Window::layout_scroll_content_size` 读取 Taffy 已计算的可达溢出，换算至
  既有 padding-excluded 滚动尺寸约定。可滚动 Div 在帧内保留自身布局 ID，
  将该尺寸与直接子盒尺寸合并；后者继续服务条目定位及绘制时决定的内容。
  这使被固定宽度包装器包住的宽表格等可沿原滚动机制到达，不需要应用
  猜测最小宽度、额外布局遍历或几何缓存。尚未编译或验证。

- `Styled::grid_cols_auto_fit(minimum)` 将逻辑像素最小宽度直接转换为 Taffy
  的 `repeat(auto-fit, minmax(minimum, 1fr))`；重复数量、空轨道折叠和剩余
  宽度分配使用已有原生布局。`GridTemplate::auto_fit_minimum` 默认不设置，
  序列化旧模板保持不变；新模板要求正的有限最小宽度，不能同时声明编号
  重复或固定首轨道。已有编号/首轨道构造器显式清除此选项，不增加应用
  侧容器宽度状态、测量或缓存。统计指标使用此入口替代窗口宽度估算。
  尚未编译或验证。

升级必须对照以上行为合并到新的匹配 GPUI crate 系列，不能只替换某一个平台版本。
这些补丁当前尚未集中编译或运行验证，也未宣称跨平台通过。

## 原生弹层类型的可访问投影

`InteractiveElement::aria_has_popup` 保存可选的 AccessKit `HasPopup` 并在
现有节点信息写入时设置弹层类型。缺省不设置任何弹层语义；不改变布局、
命中、焦点或键盘行为。供可编辑日期组合框及日历触发按钮共同使用。

## 普通文本的可选溢出换行与固有宽度

`TextStyle::overflow_wrap` 和 `Styled::overflow_wrap` 采用可选的
`OverflowWrap::{Normal, Anywhere}`；默认不设置，保留普通控件原行为。
当前应用接入点是统计表的普通文本；结构化富文本继续使用自身排版路径。
`Normal` 按 Unicode 合法断行点换行，`Anywhere` 还允许在字素和整形簇边界拆分
过长单词，LTR 与 bidi 路径使用一致策略，禁止把一个整形簇拆开。

Min-content 复用原生字形的实际 advance，按源顺序处理 bidi/字体回退；
正常换行取不可拆单词最大宽度，anywhere 取不可拆字素/整形簇最大宽度。
原生文本随后在该宽度排版，因而测量结果也能提供相符的高度与基线。
预排字形和显式间距的原子文本继续保留已有测量路径，nowrap 保持不软换行。

换行策略进入已有的文本元素和原生换行缓存键；未换行的字形布局仍共享。
最小宽度测量重用字形缓存，但其临时换行不写入跨帧换行几何缓存；
元素当前布局仍保留最后一次测量结果，不增加应用层文本测量缓存。

## 绝对弹层的延迟布局

`Deferred::with_late_layout` 为绝对弹层提供可选路径：正常布局只生成
不参与父级尺寸的绝对占位节点，在预绘制阶段、锚点已记录本帧边界后，
使用既有 `AnyElement::layout_as_root` 对子树进行一次原生布局，再交给
既有 deferred draw 管线。普通 Deferred 继续让子树参与原布局。
该入口不保存跨帧弹层几何，不增加一套绘制、焦点或动画系统。

## 原生滚动的实际偏移通知

`Context::observe_window_scroll` 使用既有 SingleSubscriberSet 和弱实体
订阅，在原生 Interactivity 完成活动项滚动与边界限制后、子树预绘制前，
通知实际发生的偏移变化。`ScrollOffsetEvent::is_from` 比较已有原生
ScrollHandle 的共享偏移所有者，调用方无需保存或复制容器几何。
现有按容器持有的 OngoingScroll 只补充上次应用的一个偏移值，首次
记录不通知；手势过滤保持原路径。回调前释放偏移与句柄借用，允许
订阅者关闭弹层并释放自己的订阅。普通滚轮输入及没有变化的位移
不发事件，不增设轮询、定时任务或应用页面缓存。


## 原生自动滚动条占位

`Style::auto_scrollbar_gutter` 与 `Styled::auto_scrollbar_gutter` 配合现有 `scrollbar_width`，只在实际溢出的可滚动轴保留 gutter。布局引擎记录启用节点及祖先分支，首次按无 gutter 计算，再根据 Taffy 的实际 overflow 逐轴启用占位；每轴只启用一次，复算复用原生布局和文本测量缓存，在 prepaint 前完成。再次计算同一棵子树时先恢复无占位，允许内容减少后移除滚动条；逻辑像素的溢出判定沿用现有滚动上限两位小数规则。不启用时不遍历额外子树，不保留应用层尺寸副本。

`Div` 从完成的原生布局读取实际 gutter，统一用于子元素裁剪、滚动上限和 `ScrollHandle` 的有效视口。滚动条区域仍属于容器自身的滚轮命中范围；直接子元素定位前先写入当前帧边界和滚动轴，避免首次展开依赖上帧状态。padding 百分比仍按原布局尺寸解析。自动占位仅对显式启用该选项的布局生效，默认覆盖式滚动不改变占位和裁剪行为。


## 跨弹层的无障碍控制关联

`InteractiveElement::aria_controls(&FocusHandle)` 用现有焦点标识声明受控弹层，帧结束时从本帧焦点节点映射解析 AccessKit controls；目标不在本帧可访问树时不导出关系。关联元数据按需装箱，帧内关联列表仅在无障碍启用时写入，不新增应用节点 ID 缓存或隐藏控件。

`aria_active_descendant_of(&FocusHandle)` 支持视觉上由触发器控制、在 deferred 层绘制的选项。原生实现核对当前焦点属于该触发器、触发器声明了 controls、当前选项确实处于受控列表的可访问子树内，再沿既有 AccessKit 活动项焦点路径报告。普通树内 `aria_active_descendant()` 保留原来的聚焦祖先约束。


## 共用 UTF-8 词典边界

`word_ranges(&str)` 直接提供字节区间迭代，使用已锁定 ICU 依赖的 `WordSegmenter::new_dictionary` 和既有 `compiled_data`，涵盖通用 Unicode 边界及其 CJK、东南亚文字词典路径。不启用额外模型 feature，不下载模型，不转换为 UTF-16，不为编辑操作保留另一份边界表；分段器借用编译数据，复杂文字的临时分段工作由既有 ICU 迭代器承担。

## 软行制表位与范围宽度

`LineWrapper::wrap_line_with_measurement` 复用既有 Unicode、字素和原生簇
边界，允许宿主提供按软行起点计算的范围宽度。换行时重新计算带入后缀，
不能沿用旧起点的 Tab 宽度；受保护组的行尾悬挂宽度也读取同一测量。
普通 `wrap_line` 和空白策略入口继续使用原来的增量路径。

`LineFragment::shaped_text_with_cumulative_advances` 直接借用测量器现有的
源序 f64 累计位置及末尾哨兵。`LogicalAdvances` 内部区分原有共享字形宽度与
借用累计位置，后者不再构建、排序或保留第二份双向段落宽度索引；续行沿用
同一张表。构造时检查完整源长度、UTF-8 边界、有序唯一索引及有限位置，
候选仍遵守原生簇、字素、nowrap 和空白策略，注册对象保留其实际宽度。
没有改变原生字体塑形、Tab 网格规则或建立常驻缓存；此改造尚未编译或验证。

`TabStops` 共用下一制表位及半个 0 字宽的最小距离规则，遵循
[CSS Text 的 Tab 定位](https://www.w3.org/TR/css-text-3/#white-space-phase-2)。
`LineLayout::source_advances` 提供临时源序原生 advance，合并同簇后保留
f64 累计；测量和绘制由同一源序宽度选择网格，避免视觉坐标相减的舍入
使 Tab 额外跳格。绘制仍使用既有 `ClusterSpacing` 同步字形、前进量与
插入点，不修改源文本或共享整形缓存。替换对象可提供实际宽度和软行
源偏移；字体占位仍由宿主替换，不将其重复计入 Tab 前进量。
`ShapedLine::source_advances_with_objects` 归并源序字形及已排序对象，
统一替换占位前进量；原生后端省略占位字形时也在实际源位置补入
对象宽度。换行和绘制共用这个流式入口，不逐字形二分查找对象。

`TabStopRun` 接收按源序排列且不重叠的字体度量区间，宿主按实际 Tab
的请求字体和塑形字号解析，并合并等价度量。区间可跨越不含 Tab 的
文本；同字体段落不为每个 Tab 保存重复度量，也不保存逐字节样式表。
`ShapedLine::with_resolved_tab_stops` 借用硬段落的区间和软行源偏移，
在共享测宽规则下调整本行字形，不重新查字体或复制区间。统一度量
入口使用单个栈上区间，原有源序校验及明确失败路径保留。

每个软行沿用完整硬段落的方向，符合
[Unicode L1 的段落内 Tab 方向规则](https://www.unicode.org/reports/tr9/#L1)。
原生后端未提供独立 Tab 簇或无法保留其源序时沿用明确的排版失败路径。
以上实现尚未编译或验证。
