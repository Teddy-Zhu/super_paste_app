# Taffy 0.15.1 限定布局补丁

来源：crates.io `taffy` 0.15.1，源修订
`0064bd54cbe38bf1d8bcd6ec6ab0704d45fb048a`。
发布包 SHA-256：`31f7bd2d97198df05186f93a3a3c306845f40e8b3c22bf9279cdd7e5cf5fea7d`。
这是 GPUI 0.3.11 锁文件已有的同一版本，不新增另一套布局引擎或版本。
保留发布包源码、元数据、示例及原有用例；没有新增测试体系。
MIT 许可，版权属于 Visly Inc. 与 Taffy Authors。发布包未附许可文件，
本目录 `LICENSE` 从[同版本上游标签](https://raw.githubusercontent.com/DioxusLabs/taffy/v0.15.1/LICENSE)补入。

## 外侧列表标记

`BlockContainerStyle` 与 `Style` 增加可选 `ListMarkerMetrics`，默认 `None`。
只存已测量的上升部基线与盒高，不持有文字、标记节点或排版闭包。
GPUI 的 `set_outside_list_marker` 将逻辑像素转换为布局使用的设备像素后设置；
Kit 仅在结构化正文的原生 Block 容器上启用。

块流在首个具有基线的正常流子块提交位置前，将标记上升部与子块自身的基线
比较，计算非负下移量；该子块的顶部位置、折叠边距和之前块的高度不抵消上升部。
子块位置、浮动高度贡献、后续块、溢出及总高度使用同一次布局的位移结果；
此前没有基线的子块保持正常位置。不通过绘制变换或第二遍布局修补高度，
也不插入新的格式化上下文或外边距引擎。无正文基线时，顶边标记的完整盒高
计入固有高度；无子节点的空块由 GPUI 叶布局桥接处理，非零占高阻止折叠穿透。
标记自身不被当作正文首行，也不占用内容列宽度。

GPUI 文字和 Kit InlineFlow 通过同一个测量结果返回尺寸与首行基线。
带标记块的固有高度查询使用原有完整子布局入口，因为尺寸缓存及 Flex/Grid 的
只求尺寸捷径不保留基线；沿用 Flex 基线对齐取得最终子布局的机制，并复用同一
最终结果缓存。宽度与约束变化仍按正常布局重算，不增加绘制端补偿排版。
默认无标记路径沿用上游算法。

`Cache::first_baseline` 和 `TaffyTree::first_baseline` 只读现有最终布局条目的
基线，不克隆结果、重测或增加第二份几何状态。GPUI 换算 DPI 后交给 Kit 的
透明表格宿主，使布局与标记绘制使用相同的原生盒基线。没有最终结果或没有基线
均返回 None，不从后代文字或尺寸条目伪造。规则参考
[Blink 外侧标记布局](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/layout/list/unpositioned_list_marker.cc)。

## 原生表格行基线

Grid 宿主通过 `table_grid` 显式启用；单元格以 `TableCellAlignment` 标记，
普通 Grid 保持原语义；数据详情显式启用表格元数据，卡片仍用固定列宽 Grid。单元格要求伸展的列 Flex 与单一正文块。
GPUI 在请求布局后统一设置对齐并检查此结构，不插入额外布局节点。
`TaffyTree::update_style` 原位修改标量元数据，并执行与 `set_style` 相同的祖先
缓存失效，避免为设置表格标志克隆列轨道向量；列表指标也复用此入口。

baseline 单元格在列宽求解之后、行高求解之前取得基线，使用实际占列宽度，
包括 colspan；不使用最窄内容宽度的首行来决定最终行的对齐。读取正文块已有
完整布局与最终缓存基线；空正文不导出 Flex 为空子块合成的基线，非空但无行框
的正文回退到内容底边。对齐量仍复用 Grid 的固有高度补偿，但最终格子不使用
外边距位移：`LayoutInput::table_cell_baseline_shim` 在 Flex 提交最终布局前移动
单一流内正文块，参与原生位置、溢出、基线、命中和定位锚点计算，不改写 padding
或格子尺寸；即使跨行正文对齐后溢出，也不把边框和背景撑大。该约束进入原
`CacheKey` 的完整布局匹配；尺寸缓存忽略这个不改变尺寸的位移。不修改常驻样式，
不增加另一份正文/几何缓存。普通输入为零，转置时清零，无流内正文不合成位移。

列宽重算时同步刷新全部原生表格行，而不仅是有 baseline 单元格的行；补偿变化
清理相应高度贡献缓存，强制重算后的实际总行高变化也更新表格宿主尺寸。
自动行高先使用非跨行单元格的固有高度与对齐补偿，跨行格完整上升部约束起始行，
其固有高度不再次叠加基线位移。跨行范围按结束行、逆起始行、逆高度排序，
先处理内含范围，同范围优先较高格；额外高度优先均分到范围内后续合并起点，
否则按非空行现有高度分配，全空范围只增长末行。行高与单元格固有高度复用原
tracks/items 字段，只在存在跨行格时分配本次求解的起点标志及引用排序数组，
排序不额外申请稳定排序工作区。此路径要求 auto 行轨道，不声称已覆盖作者指定
行/行组高度或百分比；普通 Grid 的轨道分配不变。

首行使用参与单元格的最高
基线；没有参与者时，用首行高度减去单元格有效底部内边距/边框，跨行内容不冒充
后续行的位置。首个空行使用行顶，不跳到首个有文字的行。规则参考
[CSS 表格高度算法](https://www.w3.org/TR/CSS22/tables.html#height-layout)及
[Blink RowBaselineTabulator](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/layout/table/table_layout_utils.cc)。

`LayoutGridContainer::table_cell_content` 仅在显式表格路径读取已有正文布局；
内置 `TaffyView` 已提供实现。自定义树启用此路径时也必须提供，不静默合成成功。
合并边框的半宽度量由 GPUI 桥接写入现有边框样式，绘制由应用宿主管理，
不在 Taffy 里另存边线几何。这不是完整 CSS table 布局引擎；作者行/行组高度
和百分比、特殊空内容及约束/DPI 组合仍待补齐或人工对照。

## 维护边界

这是受控富文本文档的普通块流支持，不是完整 CSS `::marker` 实现。
不新增 inside 标记、任意作者生成内容、垂直书写或浏览器浮动列表的完整兼容承诺。
应用当前文档模型不接入作者浮动；实际嵌套边距、空块与 DPI 组合尚待人工对照。
表格的限定行基线已传到 Kit 绘制端；不据此宣称完整表格模型或浏览器复刻完成。

升级时一并审阅 `style/block.rs`、`style/mod.rs`、`compute/block.rs`、
`style/grid.rs`、`compute/grid/`、`compute/flexbox.rs`、`compute/leaf.rs`、
`tree/layout.rs`、`tree/traits.rs`、`tree/cache.rs` 和 `tree/taffy_tree.rs`，
以及 GPUI 的叶测量/空块/最终基线与表格元数据桥接，
Kit 的正文 Root、InlineFlow、透明基线宿主与标记预绘制。
标记和单元格元数据默认 `None`，表格策略默认 false，正文位移约束默认零；
不要给普通容器附加这些指标。
源代码实现尚未集中编译或运行验证，不宣称跨平台或浏览器复刻验收通过。

## 原生表格自动列宽

`table_auto_columns` 默认关闭，并要求 `table_grid`、原生单元格、
自动轨道或定长宽度偏好。定长偏好影响最大内容约束，不能代替固有最小宽度；
自动轨道复用现有 min/max-content 缓存。最小宽度至指定宽度区间增长定长列，
指定宽度至最大内容区间增长自动列；超出最大内容后优先按自动列最大内容
宽度比例分配余量，没有自动列时分给定长列。复用轨道 base/growth 字段，
不保存第二份列宽向量。

此路径不能用已知 width 的普通 Grid 计算尺寸捷径跳过单元格约束；最终表格
边框盒至少容纳已分配列宽。固有测量不带暂定列宽或行高，跳过会将已伸展
尺寸重新读作固有尺寸的普通 Grid 反馈探测。行高、基线、布局缓存及后代
溢出沿用原生路径。跨列单元格在单列固有约束之后按跨度及原顺序处理；
分别补足所占列的最小、最大内容宽度，内部沟槽计入已有宽度，仅真实列
接收缺额，末列吸收浮点余量。只为跨列单元格分配临时索引列表，不复制
单元格或新增列宽缓存；行跨度与列约束独立。作者百分比列和 colgroup
分配不在此限定路径内。
余量分配参考 Blink `DistributeInlineSizeToComputedInlineSizeAuto`；跨列固有
约束参考 [CSS 2.2 自动表格布局](https://www.w3.org/TR/CSS22/tables.html#auto-table-layout)。
尚未编译或验证，浏览器实际列宽仍需人工对照。

## 原生表格单元格的 max-width 与分配宽度

原生单元格的 max-width 参与固有列宽约束；列宽分配后，其盒子仍填满整列。
Grid 单元格放置与列 Flex 单元格的两处尺寸计算共同遵守该规则，
避免将内容上限变成背景/边线缺口或额外的内边距宽度。
`FlexboxContainerStyle::is_table_cell` 默认 false，Style 仅从已有单元格元数据
导出该值；复用现有布局缓存与尺寸状态，普通 Flex/Grid 的行为保持不变。

应用据此把复制统计首列的 240px 放在带内边距的单元格上，而非内部内容框。
规则参考 [CSS Tables 3 单元格固有尺寸及列宽分配](https://drafts.csswg.org/css-tables-3/#computing-cell-measures)；
浏览器与原生实际布局的一致性仍以项目的人工验收标准为准。
