# Kit 0.9.0 scoped patches

`gpui-component/` and `gpui-base/` are their crates.io 0.9.0 sources, licensed
Apache-2.0. Package manifests, licenses, notices and upstream tests are retained.
The workspace fixes Kit, Base, Component, assets and macros to 0.9.0; updating
one layer requires reviewing the family together.

## Contextual repair at soft-row boundaries

Structured inline paragraphs with the strict native-layout guard reuse the
existing native font-run boundary API
when a chosen soft-row cut is inside a shaping cluster. Only actual unsafe cuts
are added; the complete hard-paragraph source, isolates and font attributes are
shaped together again, then wrapping uses the resulting advances. Cuts which
move inside a later row are removed before accepting that layout. The original
shared hard-line string is retained across passes. Tab measurement, no-wrap
ranges, space policies and source-relative glyph positions stay on their existing
paths. No global ligature switch, per-row isolated shaping, source marker or new
layout cache is introduced. macOS/Windows continue using their existing upstream
font-run boundary mechanism; Linux retains the adjacent-source context adapter.

Unwrapped/single-row and unguarded ordinary-text paths retain their existing
behavior. A glyph scan matches chosen cuts without allocating a second bidi index. A
repeated unsupported native cut fails explicitly. Eight additional shaping
passes cap changing-wrap feedback; exhaustion reports a native shaping work
limit through the existing document guard. Native visual acceptance remains
manual, and platform build results are recorded in the project README.

## Live reduced-motion preference

Base retains one GPUI subscription on macOS and Windows and reuses its existing
preference reader and application-override policy for native notifications.
Following starts before the initial read. Linux subscribes to Portal changes
before reading the initial value; a failed initial read retains the subscription,
and failed subscription setup still allows the initial preference to apply.
No polling, new animation driver or additional thread is introduced. Native
macOS/Windows delivery and visible animation behavior remain unverified.

## Numeric inline alignment

`InlineBaselineOffset` lets a structured host retain the computing font and
viewport basis for a signed length calculation. `InlineBox` resolves it with
its existing geometry; descendants, selection, backgrounds and decorations
consume that same shift. Atomic images support a baseline offset through
`ImageVerticalStyle`. Existing constructors supply no numeric offset.

Font and viewport metric descriptors expose signed `f64` lengths before the
consuming property's range is applied. Decoration thickness keeps its existing
positive used-value clamp. Only flows whose scopes or atomic images depend on
viewport units add viewport dimensions to their geometry key; resizing cannot
reuse stale geometry, while other flows retain the existing reuse path. No new
font, layout or raster cache is introduced.

## Linux compilation integration

The independent workspace passes `cargo check --workspace --locked` on Linux.
Earlier section notes preserve the implementation-stage validation status;
runtime behavior and other platforms remain unverified.

Fixed Taffy 0.15 auto-track constructors and direct content alignment. Native
table baseline calculation retains two scalars before detailed grid information
takes ownership of the tracks, without cloning the track vectors. The WGPU
font-kit dependency enables its `source` API, and Wayland title setup borrows
the title before the window retains it.

Immutable inline geometry and metrics use `Arc` so shared parsed documents
remain transferable to the existing background parser. The metric map uses
opaque address keys for sources retained by its entries; no address is
dereferenced and no unsafe thread-trait implementation is added. `VirtualList`
forwards GPUI interactivity to its existing stateful base, without another
wrapper or hitbox. Input placeholder reads borrow the model directly, while
composition state is read from the input model. Component adapters use native
executor timers, `Nearest` scrolling and the existing style extension trait.

`editor_host::FlowPosition::at` accepts both owned and borrowed block slices,
matching the existing internal flow calculation. Structured file rows and table
cells can render their existing paragraphs without a temporary vector of
references on every frame. This does not copy or reparse the document.

Structured flows inherit an optional `TextStyle::tab_size`. Only
layouts containing actual tabs query font-space and half-`0` advances. Prepared
rows apply the native tab-stop adjustment to each complete native soft row before
splitting paint contexts, so sizing, painting and selection retain the same
geometry and source indices. The existing per-flow typography/font-generation
key owns invalidation; no tab cache or second text representation is retained.
Unavailable native tab cells use the existing native-layout failure path rather
than source-space substitution. Wrapped flows share the source-range measurement
path described below; input controls retain their own settings. This patch has
not been compiled or verified.

## Structured block flow and fills

`BlockNode::Root` retains an optional authored background. Colored groups paint
the fill behind their complete content width, with the final flow gap outside
the fill. Roots may additionally retain an authored `BlockFlow`; compaction
preserves both colored and authored-flow roots, including empty margin struts.
Selection/copy and range-index traversals add no separator for
roots, regardless of their fill or compaction. Child blocks retain their own
breaks, so recoloring a group cannot change copying. Following siblings find the last visible
block through roots without flattening or copying the tree. `Heading` has an
optional content fill, inside its existing surrounding flow gaps. HTML and
Markdown constructors retain `None`; existing fixtures supply that field.

The application clears block fills from descendant inline highlights and uses
these native groups for paragraphs, containers, quotations and lists. List item
fills stay in the content column, leaving outside markers unchanged. Native
tables paint their own fill separately from captions; existing row/cell layers
remain in use. Structured transparent blocks retain their source box and count
toward the adapter node budget. This uses the normal
GPUI background quad and the existing document/selection/resource owners.

`TextViewStyle::with_authored_flow` disables Markdown paragraph, heading and
rule spacing for the structured host, including anonymous text runs. Parsed
Markdown/HTML constructors keep `flow: None`, and the style flag defaults to
false. The desktop adapter resolves source block margins in their own font
context and uses the existing Taffy block layout for margin collapse. Direct
first-element top margins follow the reference preview's reset. Captions and
tables establish independent formatting contexts using a single block child
inside a grid item, without introducing overflow clipping. Table cells likewise
keep all their blocks in one alignment flex item so sibling margins can collapse
while remaining inside the cell.

Authored lists put padding on the list box and position their shared marker
outside the normal block-flow item. They no longer wrap each content block in
a separate flex row or clipped container. The Markdown checkbox/spread path is
unchanged. Empty marked items retain a font-relative line strut. No additional
parser, document string, selection entity, identity table or layout cache is
introduced. Exact marker baseline alignment and empty-item edge cases still
require implementation/visual review.

`HorizontalRule::appearance` optionally retains a `RuleStyle::Inset` tone pair.
The application resolves the UA gray foreground and authored color resets, then
computes the two tones once using the browser's luminance/brightness rules
([border shading](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/paint/box_border_painter.cc),
[color adjustment](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/platform/graphics/color.cc)).
Native drawing uses two adjoining filled paths for the one-pixel inset edges
and corner joins, retaining alpha and exposing the block background through
transparent borders. It creates no bitmap, texture or additional cache. Default
Markdown rules retain their existing hairline. Actual tone, joins and DPI
snapping remain unverified.

Paragraphs without parser spans no longer use the same literal element ID for
all inline flows and image animation scopes. An explicit editor key wins;
otherwise identity comes from the existing retained selection owner's address.
Parser span IDs stay unchanged. This introduces no identity registry, extra
entity, text copy, shaping pass or cache. Table section/row solid fills stay
with each originating cell across its row span; missing cells receive no row
fill. This matches browser table painting, rather than painting full-width
row bands over spanning cells. Inline background boxes remain incomplete;
platform drawing and the authored margin integration remain unverified. These changes have not been
compiled or tested.

## Structured decoration styles

`TextMark` can retain a shared `DecorationStyles` record independently of glyph
foreground. Structured document adapters supply the decorating element's native
underline/overline/strikethrough styles; ordinary descendants retain those strokes when
their foreground or font changes. The record includes color, thickness and GPUI's typed stroke pattern. Identical records are retained, and marks
without supplied strokes allocate no record. Mark merging and `mark_highlight`
carry the optional complete strokes into GPUI's existing decoration styles,
including its existing fade and layout paths. GPUI's stroke structs add a defaulted
pattern while retaining the legacy underline wave flag; complete local literals
supply the new default. Positive widths use its per-window physical
pixel snapping; the application keeps CSS zero/negative thickness positive at
the logical representation boundary so the one-device-pixel minimum applies.
`DecorationStyles::parent` retains independent outer decorating boxes; each node
contains only its own strokes and metric sources. The host reuses ancestors
across leaves instead of retaining flattened ancestor arrays, and rebases all
propagated origins at block boundaries. A supplied source chain is authoritative
when highlights combine, so absent head slots do not duplicate ancestor lines.
Record equality preserves box identity, preventing equal-looking sibling boxes
from coalescing into one line; `set_decoration_styles` compares values separately
to retain an unchanged current record and omit the empty default.
This does not add a drawing cache or shape glyphs again. These changes are unverified.

GPUI underlines also add a defaulted `skip_ink` flag. Structured document adapters
retain that policy with the complete stroke; TextView mark and link underlines
explicitly enable the native auto policy. Input/diagnostic complete literals keep it disabled.
Native ShapedLine/WrappedLine painting borrows the source text for CJK handling
and reuses the glyph draw's raster bounds before reading any contour. The backend
and clipping changes are recorded in `GPUI-PATCHES.md`; no additional TextView
cache, shaping pass or source copy is introduced. These changes are unverified.

Window-dependent decoration geometry retains an optional shared `DecorationMetrics`
record in `DecorationStyles::metrics`. Its thickness slots share an `Arc<DecorationMetric>`:
either one element's `DecorationFont` (family, weight, style, size and typed font
metric), or a viewport axis and authored percentage.
`InlineHighlight` carries the same record through merging and fades; replacing a
stroke also replaces its metric source. `from-font` reads the recommended width;
`ex/cap` read existing font metrics, while `ch/ic` use the existing window layout
cache for their single unit character, including native font fallback/features.
The body is not reshaped, and no separate metric cache or text copy is added.
Missing metrics use the fallbacks specified in [CSS Values 4](https://www.w3.org/TR/css-values-4/#font-relative-lengths).
The host retains a computed font-length source for explicit inheritance, even
when that element draws no line; percentages and the from-font keyword still
resolve on each newly decorating element. Ordinary pixel widths allocate no
thickness descriptor. Equal stroke requests reuse an already resolved width.
`used_decoration_thickness` applies the common lower bound and saturates finite
products to the [signed 26.6 CSS layout range](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/platform/geometry/layout_unit.h)
before f32 conversion; per-window device-pixel snapping remains authoritative.
The application resolves `rem` against the reference page's 16px root and
`lh` against its controlled, inherited line-height. Computed absolute lengths
stay f64 through explicit inheritance, including `em`, until the native used
width boundary. `rex/rcap/rch/ric/rlh` retain the platform root font at 16px;
they reuse the same metric path and disable host body features/fallbacks.
`rlh` reads the first font's cached ascent, descent and line gap, independently
of fallback glyphs in the body. Heading refinements explicitly preserve the
preview's line-height, so measurement and drawing share the same basis.
Browser-specific normal-line-height rounding and metric differences still
need comparison. These changes are unverified.

Each stroke also optionally shares a `DecorationOrigin`: its decorating box's
font family, weight, style, size and absolute upward baseline displacement.
An ancestor's line therefore retains its baseline/font when descendants change
size or use subscript/superscript. The host rebases propagated origins at block
formatting boundaries and uses the same displacement calculation as InlineBox.
Run construction subtracts the decorating displacement from the current scope's
displacement and sets GPUI's optional `baseline_offset`. Replacing a stroke
replaces or clears both its thickness and position sources; fades preserve both.
Underline uses the auto gap below that baseline; overline and strike use the
source font's ascent, without shaping text. One borrowed last-font/ascent entry
exists only during a run construction, shared across stroke types and adjacent
highlights; it retains no fonts or geometry across layouts/windows/frames.
The host's auto thickness is font-size/10, also used when from-font has no valid
recommended width. This follows the reference [Blink offsets](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/layout/text_decoration_offset.cc)
and [thickness](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/style/text_decoration_thickness.cc)
policies; exact platform/browser placement still requires comparison. Ordinary
native/Markdown controls have no origin descriptor and keep native positioning.

Viewport thickness resolves from the window's current logical client size in
the same run construction, using f64 until the existing saturation/snap boundary.
The application maps width/height, inline/block, min/max, and their small/large/
dynamic variants to the four native axes: the supported desktop host has no
retractable browser chrome and the controlled document has horizontal writing.
The original sanitizer and application styles create no size-query containers;
`cq*` therefore use the small viewport as specified by [CSS Containment 3](https://www.w3.org/TR/css-contain-3/#container-lengths).
This is an explicit scope of the host adapter, not a generic query-container API.
Retained flow fragments carry the shared descriptor into their per-frame Inline
layout root, so height-only window resizing updates strokes even when wrapping
and column widths are reused. Ordinary lengths and typography do not acquire
new viewport cache keys or listeners; viewport-only requests do not resolve a
font. No document rebuild, size cache, duplicate shaping pass or source copy is
introduced. The existing geometry, merge, fade, wrapping and selection paths
remain in use. Native resize and DPI behavior are unverified.

Overlines use the same shared `DecorationStyles`/`DecorationMetrics` records and
GPUI highlight/run pipeline as the existing strokes. Merge, inline-code style
projection and fades retain them; `from-font` reuses an already resolved
underline/strike thickness when the source font matches. `DecorationResolver`
resolves the source parent graph once per run construction, keyed by source
allocation and fade bits; it owns no state beyond that call. Equal ancestors
share resolved native nodes, each retaining the original source allocation as
stable identity rather than allocating an identity token. Native layers compare
identity and resolved stroke values, preserving retained layout reuse across
frames while invalidating changed viewport/font geometry. The current fragment's
baseline displacement is carried separately, so shared ancestors need no copies
for differently shifted children. Head-only native overrides retain outer nodes;
fades cover every node, without fading head colors twice. The resolver's borrowed
ascent entry and existing font/window caches serve all nodes. No separate text
layout, native callback, persistent geometry cache or source serialization is
added. The host's existing document-depth and native-preview limits still apply.
These changes are unverified.

## Native paragraph direction

`Paragraph::with_direction` supplies an authored native base direction independently
of physical alignment; paragraph equality includes it, while ordinary Markdown
inherits its enclosing style. Plain text and InlineFlow share GPUI's direction
parameter and existing layout caches.
Structured flows with sorted, disjoint, optionally sparse text marks can shape a
hard paragraph, including source atoms, with GPUI sized runs before splitting complete
soft rows into visual pieces. Native clusters must cover required text cuts; only
registered atoms and omitted format controls may own source-only tails. Legacy
code boxes and unsupported visible-cluster cuts retain their existing paths. Adjacent nowrap marks
coalesce across styles and restrict break candidates on the complete shaped
paragraph. Its original source supplies Unicode line/grapheme boundaries and
its existing glyphs supply advances, including ligatures across nowrap edges.
Protected spaces retain their width. No fabricated text or glyph is inserted
to impose nowrap or allocate source atoms.
Sparse object entries provide the measured advance at each actual source atom;
ordinary text continues to use its native advances. The single-pass wrapper
does not repeat source indentation on continuation rows: flow placement has
no continuation-indent field, and CSS pre-wrap does not repeat it. Contexts
are derived once per layout. The existing run builder supplies inherited body
attributes for unmarked gaps; resolved paint runs are indexed once and only
overlapping runs are sliced for each hard-line context. Paint ownership cuts
merge with child cuts in source order without a second boundary table or
artificial font-shaping break. Colors and decorations must respect real glyph
clusters; nowrap alone adds no ownership cut. Reversed, overlapping, out-of-range
or non-UTF-8 mark boundaries are rejected. Complete native
advances let the wrapper consume all soft rows in one traversal. Physical
positions use visual rank, while child registration, copy and accessible reading
order retain source order. A fragment rebases its default size and font extents
without resizing or reshaping its glyphs. Unchanged vertical metrics retain the
shared layout. Single-piece rows retain their layout; ordinary visual order
needs no positions table.
Native-omitted prefixes and cross-context tails from a conservative formatting
control whitelist retain their source in zero-width pieces. Their physical
edge follows the paragraph cursor's visual source runs, while registration
retains source order. `ShapedLineCursor::visual_source_runs` borrows the existing
L1/L2 run scratch and source-order index; querying a just-consumed soft row
reuses its prepared range. No second source ordering table is built. Distinct
embedding runs are not merged even when they share a font context. Source-run
boundaries may split a native cluster only if the remaining source consists
entirely of whitelisted controls. A backend that keeps visible source in a
ligature across embedding levels retains the native cluster; independent source
anchors for that combination remain incomplete. Completely glyphless paragraphs must have zero native width. A
control-only piece resolves its own requested font metrics rather than a
neighbor's aggregate extents. Printable source, combining marks and soft
hyphens cannot use this exception to truncate a native ligature. No additional
paragraph source copy or persistent source index is added; final source slices
retain the existing cursor ownership. A cursor defers its reordered glyph and
grapheme indices until glyph slicing actually begins, so a source-run query
alone builds neither. Before that, boundary checks use a grapheme cursor over
the complete source; an existing index is reused once available. These data
are local to layout, using the same paragraph analysis rather
than a separate parser or global cache. The application resolves HTML automatic
direction and physical alignment together, uses LTR for ordinary rich containers,
and applies the same rule to cards, details, filenames and data cells. This does
not establish verification of inline isolation or line-specific bidi reshaping. Explicit block
directions now use the same complete-paragraph shaping path as plaintext. The
adapter validates each paragraph, including headings, rather than rejecting a
direction change before inspecting its runs. Changed-direction descendants retain
this check even after returning to LTR. Inline direction attributes now provide native isolate ranges with explicit
ancestry and source mapping. These changes are unverified.

`Paragraph::with_logical_alignment` supplies `LogicalAlignment::Start/End` and
the containing block's neutral fallback direction. Each hard paragraph resolves
its direction once; soft rows retain it. `TextDirection::detect_fragments` reads
borrowed fragments and carries Unicode isolate depth across them. Neutral glyph
contents keep UAX #9 P3's LTR default, while logical alignment retains the preceding
line's direction, or the supplied block direction on the first line, following
[CSS Text](https://www.w3.org/TR/css-text-3/#text-align-property). Both alignment
and fallback participate in existing flow typography keys. No joined source or
additional geometry cache is created.

The adapter applies this to `pre dir=auto` and clears plaintext at descendant
block boundaries. Controlled paragraphs use the actual native shaped clusters
for capability decisions, including mixed sizes, baselines and inline atoms.
The former static `supports_plaintext_direction` gate is removed. Headings and
ordinary paragraphs share the same native construction and logical alignment.

`TextView::require_native_layout(message)` opts a document into strict native
ownership: unsupported sources, unsafe native context cuts and unsafe soft-row
cuts retain a typed failure in the layout cache. Fresh and cached failures both
notify the owning view and display the host's message rather than independently
reshaping text fragments. Feedback belongs to a source/font/style/width generation;
old deferred failures cannot replace a new session, and a dropped weak owner is
ignored. Font, document, style, width or explicit resource-layout invalidation
retries native layout. Entering the failure state clears stale source selections.
A single context without source-only anchors retains its complete native row,
including bidi ordering, instead of copying glyph slices for each visual run.

`MarkdownExtensions::block_renderer_with_context` supplies the owning document's
`editor_host::RenderContext` to structured custom blocks. HTML table cells clone
and refine this context, retaining native failure policy through nested tables
without adding TextView entities or reparsing HTML. Existing leaf-only block
renderers keep their original callback API. Setting or clearing runtime range
highlights retires the previous native layout guard: changed paint ownership
must not inherit a stale failure, and deferred work belongs to its old session.
Typography and soft-row cuts across real ligatures remain a constraint;
paint-only cuts use the retained-cluster path below. Inline isolation uses
original-source scopes.

`text_system/cluster_slice.rs` retains paint-only parts of a native cluster.
`cluster_caret_x` accepts only actual cluster endpoints and retained native
insertions; `slice_cluster_for_paint` copies only that cluster, assigns the part
its advance cell, rebases the existing glyph positions/carets/source context,
and records physical inner ink cuts on `LineLayout`. Ordinary layouts carry
no mask allocation. `paint_line` applies the mask around both direct and
prepared glyph painting, retaining the original raster identity and outer
overhang. Narrowed cluster cells bypass the approximate glyph cull, so an
overhanging sliver is not discarded before the real raster bounds are known.
Retained pieces also keep exact zero-advance cells instead of expanding
decorations or backgrounds into synthetic font-sized cells.
The contextual flow separates only boundary clusters from surrounding text,
orders their owners by source bidi runs, and consumes each original cluster
once. `ShapedLineCursor::skip_until` advances ownership and metric remainder
without constructing discarded source, glyph, caret or decoration copies.
The scoped sized/isolate shaping entry merges paint-only font runs; existing
ordinary and editor entry points retain their previous shaping policy. Native
font-run contents already distinguish layout cache keys; no parallel cache or
new global glyph index is added. Cuts inside a source grapheme keep a native
font boundary. This is not a line-break reshaper: soft-row/font-box cuts,
missing insertions and clusters crossing source embedding runs remain guarded.
Paragraph clipping recovers the exact parent chain of an actual boundary
opener and otherwise excludes already-closed ancestors; real empty children
retain their ancestry. These changes have not been compiled or tested.

WGPU assigns the scoped Cosmic `Attrs::shaping_span` identity to each native
font run and all its fallback attributes. Compatibility no longer merges
explicit same-family typography cuts; the glyph metadata remains a font id
and no epsilon size is introduced. Harfrust receives borrowed pre/post source
for those runs, using its existing bounded context and buffer reuse. Optional
word caching is bypassed for contextual spans because its key lacks adjacent
source; the existing GPUI paragraph cache remains authoritative. Ordinary
zero-span Cosmic callers keep their prior policy. See `cosmic-text/PATCHES.md`
for the bounded-context and cache contract. This foundation is wired into
current font/size shaping; soft-wrap shaping remains incomplete and these
changes have not been compiled or tested.

`TextIsolation` and `shape_line_with_sized_runs_and_isolates` provide a native
path for properly nested isolate ranges, including empty scopes, within one bidi
paragraph. Explicit parent indices distinguish empty descendants from adjacent
siblings at the same original byte. The existing line-cache key includes the original ranges. On a
miss, `IsolationSource` builds a presentation source with LRI/RLI/FSI and PDI,
adjusts native font-run lengths and restores native glyph/caret indices to
original UTF-8 offsets in place. Clusters containing only inserted controls
are excluded from paint; the authority source and decoration runs are never
rewritten. Zero-source isolated lines retain ordinary native empty-line metrics,
including when no font run exists to cover inserted controls. Plain lines keep
no isolation context or presentation allocation.

Presentation source construction sweeps opening-ordered scopes with a closing stack, avoiding a second sorted event buffer. Paragraph separators are indexed once, and immutable boundary maps retain no spare capacity. Shared-text shaping passes its existing `SharedString` directly to the layout cache instead of rebuilding it from `&str` before every lookup.

`LineLayout::isolation` shares the presentation source and sparse boundary map
through native slices and soft rows. Cursor bidi resolves the full enclosing
presentation paragraph, partitions the presentation source after the last
closer at each original row boundary, then maps nonempty visual runs back to
original offsets. Empty scopes keep their full enclosing control topology.
It must retain the control topology: removing controls before L2 would merge
adjacent equal-level isolates and reorder their contents together. Default
cursors inherit the isolation layout's native base direction. Length overrides
retire the context when they change source length; ordinary font-metric and
tab-advance adjustments retain it. Structured paragraphs now collect scopes
from their existing inline-box ancestry. Direct nearest-isolate links skip
intermediate typography boxes without a second retained scope tree. Collection
merges absolute empty-box anchors into source order, and clip them across hard lines and Unicode paragraph
separators. Binary search and parent links recover enclosing scopes without
rescanning the whole list for each line. Replaced content retains its actual
U+FFFC source position and inherits only its parent's strut in an isolation
wrapper. Ordinary content skips scope collection. HTML automatic direction
is resolved for the whole element, then retained across its paragraph fragments;
the general native API still supports FSI for other callers. Flow direction
reads the native presentation source through
`ShapedLine::detected_paragraph_direction`, including original bidi controls.
The adapter no longer preemptively rejects inline dir. Deferred collapsed
whitespace stays before empty nodes and both images and image placeholders,
using an in-place child rotation only when empty anchors intervene. These
changes have not been compiled, tested or compared visually.

HTML direction semantics follow the [dir algorithm](https://html.spec.whatwg.org/multipage/dom.html#the-dir-attribute)
and [default bidi rendering](https://html.spec.whatwg.org/multipage/rendering.html#bidi-rendering).

## Native paragraph justification

`Paragraph::with_justification` routes plain and mixed text through InlineFlow.
Only soft-wrapped rows expand; final rows and rows before a forced break retain
start alignment, following [CSS Text](https://www.w3.org/TR/css-text-3/#text-align-property).
The flow finds word-space and CJK expansion opportunities using the existing
Unicode line-break/script properties, graphemes and native shaper clusters. It never
inserts padding characters or reserializes rich text. Atomic images retain their
measured boxes. Word spaces own trailing expansion; CJK boundaries belong to the
preceding ideograph's trailing side, or the following ideograph's leading side
when the preceding item is Latin text or an atom. Both sides of one cluster are
merged into one `ClusterSpacing` entry. Backgrounds and selection cells include
the expansion, while ink and interior carets move inside that same cell; logical
sides swap physical sides in RTL. Code backgrounds grow with their text.
Collapsing ASCII spaces lose advance at a line end; NBSP and
preformatted spaces are not collapsed. Wrapping fit checks use the same trailing
space policy and preserve code padding.

Spacing is shared with StyledText, whose adjusted glyph positions also drive
selection, reveal backgrounds and link hit testing. Paragraph equality and the
existing bounded flow/retained-layout keys include justification and spacing;
unchanged shared spacing avoids scanning its entries again. There is no second
geometry cache or copied document. Text-only flows reuse the existing paragraph
source/mark cache and paragraph selection owner, including authored-size and
no-wrap flows. Full-range shaping inputs share their source string allocation;
source/reference links and transient fades remain resolved at render time.
Fragment text styles are resolved against the
active window style, including an explicit left alignment after row positioning.
This does not complete wrapped bidi/isolate layout or establish browser-specific
language/font parity. These changes are uncompiled and unverified.

## Native editor keyboard caret ownership

Source-order Left/Right, word motion, End/document-end and their selection
extensions carry typed native caret affinity. Arriving at an end retains the
upstream side; arriving at a start retains the downstream side. The column
anchor uses that same hit and soft-row ownership. Soft boundaries are queried
by binary search in the existing wrap map, including rows outside the viewport;
no text conversion or second caret/layout cache is added. A no-op horizontal
or word move keeps the cursor's existing affinity, and deselection retains its
head's affinity when the source position is unchanged.

Grapheme/token clipping, source-order range extension, undo boundaries and
multi-cursor merging retain their existing paths. This does not implement visual
horizontal traversal through bidi runs. These changes are uncompiled and unverified.

## Native selection cells and endpoints

Inline selections, range backgrounds and reveal geometry consume native source
clusters and physical row advances. Combining marks share their cluster's cell;
one logical range may paint several disjoint rectangles in bidi text. Soft rows
use the same alignment and advance origins as native painting. Newline cells
keep selected empty rows visible. Backgrounds and selection share their temporary
cells within a paint, and selected rectangles merge in place without another
full output buffer. No persistent geometry cache is added.

Native clusters with interior grapheme carets expose smaller selection cells
through GPUI's `cluster_cells`. Both editable range spans and retained plain/rich
selection geometry consume those cells, so selecting part of a ligature does
not necessarily select its entire advance. Source offsets, row ownership and
bidi direction remain native; the existing temporary geometry holds the cells.
Tab expansion shifts the same native caret coordinates alongside glyphs.
No per-character reshaping or second geometry cache is introduced. Native font
caret coverage and unresolved cases are documented in GPUI-PATCHES.md; these
changes are uncompiled and unverified.

Paint and Copy resolve both drag endpoints through the native nearest-caret
path against the same shaped lines, replacing per-character position queries.
Native hit affinity determines which way to snap a font-fallback cluster edge
inside an extended grapheme. The existing Unicode GraphemeCursor receives the
complete source and reads the required Unicode context without precomputing a
boundary table.
Immutable painted selection snapshots retain the alignment as well as the native
layouts. Selection handles use downstream start and upstream end affinities.
Inline explicitly preserves its source through native layout: inherited
ellipsis/native clamps are cleared, while TextView continues to apply its own
whole-line clip. Its retained style key normalizes those ignored settings.
Native display affixes therefore cannot turn selection/link offsets into byte
positions in a different UTF-8 string.
Wrapped paragraph reshaping, missing native caret stops and selection across
all inline objects remain incomplete. The rich adapter retains runtime native
cluster ownership checks. Current implementation scope is recorded in README;
these changes are uncompiled and unverified.

## Editable caret and range geometry

Input selections retain the native `TextHit` independently of the existing
soft-wrap row policy. A source-offset stamp invalidates a stale side when the
head moves; explicit placements assign the side, and merges preserve the active
head. Mouse/touch placement and drag, vertical movement and selection, additional
cursors, preferred columns, reveal and IME positions consume this state. Masked
display offsets are mapped through the existing character-based mask projection.

Input selections, search and range decorations visit native cluster cells.
Disjoint spans merge within a row; frame contours follow exposed union edges
without joining across unselected gaps or stroking internal row boundaries.
The common connected single-span-per-row path retains its existing contour.
Menu hit testing, range bounds and IME first-row rectangles share these cells.
All geometry is temporary and limited to the current visible layout. Reading
selected text borrows its UTF-8 rope slice without a UTF-16 round trip.

UI caret stops and ordinary backward/forward deletion use extended grapheme
boundaries. `GraphemeCursor` consumes borrowed Rope chunks and requested prefix
context, including RI parity, Indic conjuncts and emoji ZWJ sequences. Atomic
tokens delimit independent segments; source and IME APIs remain character-exact.
An interior source caret expands ordinary deletion to the complete grapheme.
Line-edge deletion and line selection keep CRLF intact. Fresh word/line selections
reset direction and column anchors; vertical collapse starts at the selection
edge without an extra character step. Native trailing hits select the preceding
grapheme on double-click/long-press, including at EOF. Their subsequent drags
extend by complete words and preserve the resulting endpoint's native side.

Word selection walks the connecting characters without a fixed 128-character
cutoff. Word navigation no longer round-trips through UTF-16 or flattens the
whole prefix/suffix: LF boundaries limit Unicode segmentation to the relevant
logical line, preserving its whitespace/combining-mark context. A single-leaf
slice is borrowed; a multi-leaf line still needs a temporary string, so this is
not a streaming word segmenter for arbitrarily long lines. Symbol range queries
accumulate byte offsets without constructing substrings. Paired delimiter
deletion uses the existing source-range deletion/undo transaction without a
UTF-16 range round trip and falls back when it would split an attached grapheme.

Wrapped paragraph shaping and editor inline isolates remain incomplete. Native
visual movement, ligature carets and dictionary selection use the scoped patches
described below. Uncompiled and unverified.

## Multi-line editor padding

`Input::content_padding` and `Textarea::content_padding` pass optional internal
padding to Base's editor layout. This keeps text measurement, scrolling, cursor
and selection coordinates on the same padding values without adding frame
padding a second time. Existing controls keep their size-derived defaults.

## Input validation and disabled content

`Input` and `Textarea` forward optional accessible descriptions and invalid
state through `InputAccessibility` to the native editable node. The visual
frame remains presentational and does not create another focus or value owner.
The tag name exposes its current error as a description; invalid hex drafts
expose the color error. Unconfigured controls keep their previous semantics.

`dim_disabled_content(false)` opts out of the editor's additional color dimming
when the host already applies disabled opacity to the complete field. Native
disabled editing, focus and accessibility behavior remain enabled. The flag is
projected with `InputEditorStyle`, defaults to true, and covers ordinary and
highlighted text, document colors, active-line and editor background painting.
The tag editor uses one field opacity rather than dimming its glyphs twice.
It keeps the existing input state, undo, IME and layout paths; no new observer,
cache or disabled-input compatibility path is added. These changes have not
been compiled or tested.

## Structured text baseline displacement

Base `TextMark::baseline_shift` and `ImageNode::baseline_shift` carry authored
baseline displacement in body em units; a positive value raises the content.
InlineFlow splits otherwise identical text at displacement boundaries, includes
the displaced glyph/image extents in line layout, and retains its existing
shaping, selection and copy paths. Code decoration remains local to its glyph
box. Layout keys include the displacement; ordinary Markdown retains zero shift.
The application uses this together with authored font size for nested subscript
and superscript, in cards, details and table cells. These changes are unverified.

Base `TextMark::no_wrap` and `ImageNode::no_wrap` preserve source bytes and use
unbreakable measured ranges for CSS `nowrap` and `pre`; adjacent styled text and
images in such a range stay together. Normal custom-font/size runs share their
native shaped layout with GPUI's Unicode break/grapheme rules and preserve
shaped-cluster boundaries. Range measurement sums the shaper's actual glyph
advances instead of subtracting visual caret positions; offsets and reordered
glyphs therefore do not introduce negative widths. This avoids body-font width
estimates and does not treat a whole styled span as one word. Inline-code boxes
keep their existing padding and wrapping behavior. These changes are unverified.

`LineFragment::InlineObject` carries the measured width of one U+FFFC. Ordinary
images and custom objects in the inline flow use this variant; the wrapper's
existing Unicode/grapheme buffer sees the real replacement character rather
than Latin placeholder letters. Candidate offsets advance by its UTF-8 length,
and object width never requires font measurement. Continuation fragments keep
the same variant. Custom objects inherit `TextMark::no_wrap` and join the same
protected measured groups as text and images when requested.

`NoWrapText` borrows the actual measured source rather than supplying only a
length. Protected objects retain the same U+FFFC source. The wrapper coalesces
adjacent protected candidates, including shaped text with `no_wrap_ranges`,
before choosing a break. Partial group widths cannot prematurely emit a row
before an oversized group. The complete group retains its first native cluster
boundary and first non-whitespace byte offset; inner Unicode opportunities are
suppressed while the real context at both ends remains available. Source parts
stay borrowed, with no concatenated group string or new index. Continuations
preserve protection and cannot slice into a measured protected part.

`MeasuredText` likewise borrows the real source of an already-measured unit,
without joining adjacent protected units. Legacy inline-code whole spans, words
and graphemes now use it, retaining their splitting, padding and measured width.
Their edge context no longer consists of Latin placeholders and their glyphs
are not remeasured in the body font. Continuations cannot bisect these units.

A single text, shaped-text or measured-text input is borrowed directly for
Unicode segmentation; a single object uses its static source. Multiple inputs
use the existing reusable buffer. No additional source buffer or cache is added.
Inline flow requests no continuation indentation on both native and correction
paths; correction consumes only the first boundary and discards indentation.
Structured mixed flows also use whole-hard-paragraph shaping. A single text
source stays borrowed; multiple text parts and objects form one temporary shared
source with translated marks. Final child/atom cuts remain distinct from native
font shaping breaks: plain child cuts do not synthesize font-run boundaries,
while true size/box contexts and object edges retain their native boundaries.
Native lines keep the shared hard-line source; final text pieces reuse native
glyphs. Single objects and object-only non-RTL flows keep the direct measured
path, avoiding unused font shaping for ordinary image previews. Object lookup
within each hard line starts at its sorted source index. No new global source,
layout or per-character cache is introduced.

`InlineObjectAdvance` identifies an actual U+FFFC and its measured width. The
complete shaped fragment borrows the sorted sparse table; continuations keep the
same original source coordinates. Candidates consume the original native advance
but replace it with the host width at an object. Unicode segmentation borrows the
single complete source directly. Missing native object glyphs can still supply a
safe atomic cut; this does not authorize splitting a visible shaping cluster.

Inline flow resolves a complete soft row once before assigning its text pieces
and atomic slots to the original children. An atomic slot has no paintable text
line, regardless of whether the backend supplied a font glyph for U+FFFC. The
existing image/object element supplies its geometry, baseline and rendering. Only
registered source atoms may own omitted U+FFFC bytes; a literal U+FFFC remains text.
Visual ranks attach to actual fragment identities, not child-local source offsets.
Placement sorts the available ranks after justification and sums real frame
widths, retaining source child order for selection, copy and accessibility.
The application's conservative explicit-direction capability gate, inline bidi
isolation and cuts through real multi-context glyphs remain separate limitations.


Tab expansion updates the tab glyph's advance together with subsequent glyph
positions, so GPUI's caret cells and shaped-fragment widths use the same stops.
Ordinary text-row bounds sum native advances rather than subtracting logical
caret endpoints; RTL endpoints therefore cannot make a row's width negative.
This does not complete mixed-direction selection or wrapping. These changes
are unverified.

`BlockNode::List::markers` accepts shared per-item `ListMarker` metadata for a
structured host's counter label, font/color and outside indent. Markers use the
actual font's trailing space, remain separate from selectable content, and do
not alter the default Markdown numbering or item spacing policy. The app maps
CSS list styles and signed HTML starts directly to these labels.

`ListMarker::direction` controls the marker's native paragraph direction and
outside side, independently of the list container. `BlockNode::List::padding`
accepts host-resolved physical padding; `None` preserves the previous authored
indent and Markdown defaults. The app retains its reference preview's physical
1.5em left padding and the RTL UA inline-start inset rather than exchanging the
two sides. LI's UA match-parent alignment is resolved against its parent before
the item's own direction is applied. This follows the reference's CSS and the
[Chromium UA list rules](https://chromium.googlesource.com/chromium/src/+/refs/tags/121.0.6167.139/third_party/blink/renderer/core/html/resources/html.css).
Marker text stays outside selectable content and no direction controls are
inserted into source strings. These changes are uncompiled and unverified; they
do not complete inline bidi isolation or wrapped-line reshaping.

`MarkdownNode::structured` and `StructuredBlock::visit_groups` expose borrowed
native block groups to the existing selection traversal. Custom table layouts
can retain their cell blocks as the sole document representation; no flattened
text copy or per-cell TextView entity is stored. Selection styles, source
ranges, observation, text copying and clearing recurse through these groups.
Structured nodes skip the atomic custom-block selection wrapper. The editor
host's `table_cell` wrapper uses ordinary cross-cell selection instead of
Markdown's origin-cell confinement; the host retains content clipping. Copying
across groups preserves tabs/newlines, including empty intervening cells.
These changes are unverified.

## Owned asynchronous dialogs

`WindowExt::open_owned_alert_dialog` returns `root::DialogHandle`. It uses the
ordinary AlertDialog builder and defaults without changing appearance, focus,
modality, motion, keyboard handling or dismiss gestures. The application's
accepted asynchronous operation can close its own dialog after completion.
If another modal is above it, the close request waits for that modal to leave;
normal stack dismissal then processes requested closes and restores the original
focus chain. An already dismissed dialog or a handle used with another window
returns false. Completing an old task cannot close a newer, unrelated dialog.

Dialog changes are limited to Component `src/root.rs` and `src/window_ext.rs`. Ordinary dialog
callers retain their existing behavior. Application code must still validate
business input, own accepted work outside a view, and stop updating a dismissed
editor. Programmatic closure follows the existing `close_dialog` behavior;
it does not synthesize the user's OK/cancel callbacks.

## Opaque color inputs and disabled picker

Base `ColorPickerState::opaque_only` restricts color values and previews to
opaque RGB and limits the hex input to six digits. Component hides the alpha
row for this mode. This preserves the existing tag color input's RGB domain;
the ordinary picker keeps its original alpha support. No image or document
conversion is involved.

Component `ColorPicker::disabled` forwards keyboard disabling to Base and
renders a static trigger instead of a live popover. Saving a tag can therefore
disable its custom color input as well as its text fields and preset swatches.
The default remains enabled. Color changes are limited to Base and Component
`src/color_picker.rs`.

## Editable model picker composition guard

Base input state exposes `is_composing()` as a read-only view of its existing
marked range. The application's editable model picker can leave IME confirmation
keys to the input engine while retaining its own unfiltered, virtualized option
list. The patch does not change text editing, composition or undo behavior.

## Plain input UTF-16 limits

Base `InputBaseState::max_utf16_length` limits committed plain typing and paste
without splitting a Unicode scalar. Replacing a selection frees its capacity.
IME preedit remains editable and is limited on replacement commit or unmark;
the edit stays in the existing undo transaction. Programmatic values and
history replay retain their exact content. The default remains unrestricted.
The tag editor uses 20 units for its name and 100 for its description; the MCP
key input uses 256, matching the original inputs. Changes are limited to Base `src/input/base/state.rs`.

`normalize_line_endings` opts plain textarea values into CRLF/CR-to-LF
normalization before length accounting, including default and programmatic
values. The tag description enables it to retain browser textarea semantics
for pasted Windows text. Other inputs and code editor line endings are unchanged.

## Structured TextView input

Base `TextViewState::from_blocks` and `set_blocks` accept a view-owned adapter
tree and a shared source string. They bypass markup serialization, parsing and
idle parser tasks, while retaining the ordinary TextView layout, selection and
copy handling. Replacement clears previous selection projections and layout
state. Calling `set_text` or `push_str` on this state supplies literal text;
it does not unexpectedly interpret HTML or Markdown. Semantic documents belong
to the core service; each view builds its own adapter selection/layout state.
This patch does not by itself fill the rich-style/table-span rendering gaps.
`Paragraph::from_runs` constructs a fresh view-owned paragraph from structured
inline runs without concatenating and reparsing their text. The existing parser
constructor now moves its input string into the run instead of copying it.
`TextViewState::from_owned_blocks` additionally retains the host's document
owner and memory lease alongside native state; replacing the document clears
that owner. `TextMark` accepts authored foreground, font weight and font family
refinements after theme-derived inline styling, so the host can preserve
resolved document styles without flattening or serializing rich content.

`TextMark::font_style` retains an optional computed native `FontStyle`, including
Normal. Rendering applies it after the legacy italic flag and themed code
styling; merging only replaces it when the incoming refinement is present.
`resolved_font_style` supplies the same resolved face to inline-box metrics.
List markers apply the refinement after their legacy italic flag, and
`ImageVerticalStyle::parent_font_style` carries the complete parent style into
image alignment metrics. No refinement preserves the previous Markdown and
editor rendering behavior. These fields use the existing native font selection,
layout keys and geometry; they add no font cache or text-shaping pass.

`Paragraph::with_alignment` supplies an optional native left/center/right
alignment at the paragraph boundary. Unspecified alignment keeps the previous
render path; authored alignment also applies inside headings and table cells.
`RenderContext::with_markdown_extensions` propagates a host renderer registry
into nested native blocks. The desktop rich adapter uses the existing custom
block mechanism for fixed-column tables, including row/column spans and nested
cell documents; it does not serialize or reparse HTML. Table selection currently
projects the whole custom block, rather than arbitrary cell text ranges.

Structured marks additionally support a relative font size. This refinement
uses the existing inline flow's shaping, wrapping and selection geometry;
it does not apply inline-code padding, background or a monospace font. The
desktop uses it for inherited and nested `small` elements. Authored baseline
shifts share the flow's line extents, paint origin and selection geometry.
The body strut is measured once per flow layout. Retained flow typography also
includes GPUI's font-set generation, so registering a face invalidates old
dimensions even when the authored style is unchanged.

## Shared document images

The existing authoritative `TextView::image_source` and
`RenderContext::with_image_source` resolvers also cover nested table cells.
The desktop supplies opaque per-view keys, shared render images and explicit
pending/error results, with no URI/file/network fallback or new data URL.
`ImageNode::from_source` accepts original CSS geometry independently of the
downsampled GPU raster. The inline flow fits these host images to paragraph
width without changing Markdown's default inline-icon dimensions.
`ImageNode::vertical_style` supplies an atomic image's top/middle/bottom or
text-top/text-bottom alignment. Font-relative anchors resolve the parent font
at layout time, including its authored family, weight, style and size scale.
Anchors follow [CSS vertical alignment](https://www.w3.org/TR/CSS22/visudet.html#propdef-vertical-align).
Top/bottom images constrain the completed line box and are positioned after
baseline-aligned content, independently of source order. These inputs take
part in retained-layout invalidation; no additional raster cache is added.
This does not implement the corresponding alignment of nested inline text
boxes. The additions have not been compiled or verified.

An optional image visibility observer runs at prepaint using the actual image
bounds and current content mask. Host images outside the clip skip native
image layout/paint; the desktop acquires them only on entry and releases
their raster references on exit. Metadata measurement uses the original
dimensions and does not start a decoder. Failure uses alternative text.
Ordinary parsed images retain their default path when no observer is supplied.
The desktop owns request/response bounds, cancellation, revision checks and
window raster pins; this patch does not add an image cache or decoder.

## Host menu appearance and options

Base Motion exports `PremultipliedColor`, an opt-in paint value for premultiplied
sRGB interpolation with transparent endpoints. Custom menu rows, desktop card
borders and desktop controls share this value; the existing `Hsla` interpolation
policy remains unchanged. This value owns no timer or cache and uses the existing
element-scoped transition lifecycle. This patch has not been compiled or verified.

Component `PopupMenu` accepts optional item height, spacing, font and icon size,
separator style, explicit pixel padding, surface `Styled` refinements and a
one-shot entrance animation. `MenuItemAppearance` refines row geometry, normal,
hover, pressed and disabled colors, optional foreground/background/border transitions
and a press offset. Individual items may override the common appearance (the
toolbar's destructive row). Pointer state and transitions are element-scoped;
colors interpolate in premultiplied RGB, and keyboard selection does not retain
the pointer's old highlight. Custom rows use explicit pointer-presence hover,
including while pressed; down-state changes share the propagation boundary and
release-state changes run during capture before click dismissal stops bubbling.
Defaults retain the upstream theme and behavior.
The desktop uses 24 px context rows and 28 px toolbar rows while preserving
native focus, dismissal and keyboard navigation. `option()`
registers a checked or unchecked custom-handler item in the existing accessibility
toggle set; authored tag icons retain their colors without duplicate checkmarks.
`set_item_disabled()` updates task-dependent availability without rebuilding rows
or replacing menu/submenu focus. `rebuild()` also clears obsolete checkable indices.
Submenus open immediately on hover. Optional `dismiss_on_pointer_leave()` uses
one root-owned timer to close the entire chain 150 ms after leaving every pane;
entering the main menu or any descendant cancels it. Pane identity makes a late
leave notification from the old pane harmless. Dismissal or dropping the menu
cancels the timer. In-place rebuilding retains an existing close deadline; the
root reconciles its owner against the active chain so a retired pane cannot
leave stale ownership behind without a final mouse-leave callback. Disabled
submenus do not open.

Optional `pointer_submenus()` uses the main surface edge and pointer entry Y,
with overlap, a viewport margin and a pointer offset. Click/keyboard opening
uses the main surface top. The observation canvas is expanded from the padding
box to the resolved border box for positioning and parent click-inside checks.
`estimated_size()` counts authored fixed-height slots
(including separators), matching the source menu's positioning rule without a
second layout tree. The desktop clamps its context root to a 10 px margin and
flips/bottom-clamps the 120 px submenu; a custom 10 px chevron in a 12 px slot
and single-line label ellipsis preserve the original content geometry. Toolbar
menus instead anchor to their actual 28 px trigger container with a 6 px gap,
following layout changes and overlay slide motion without cached bounds.
Host-positioned panes use GPUI `keep_position()` to avoid a second fit pass.
These additions have not been compiled or validated.

`PopupMenuAppearance` groups the surface, separator, common row and submenu
indicator styles. Construction and `set_appearance()` use the same projection;
the latter refines the existing styles without replacing content or touching
focus, selected indices, scroll handles, entrance progress or leave deadlines.
`set_item_appearance()` updates an authored override, and `submenus()` exposes
borrowed child identities for recursive host styling. The desktop changes an
open menu only when its resolved light/dark variant changes, including the
toolbar's destructive row and every child pane. No polling, palette cache or
content rebuild is added. Foreground and border colors on toolbar rows retain
the source's 100 ms transition. This path has not been compiled or validated.

`WindowExt::open_owned_dialog()` exposes the existing scoped dialog lifecycle
for preview content, alongside the alert variant; `DialogHandle` is re-exported.
The wrapper preserves Kit's default shell, focus, dismissal and animations. The
application cancels detail readers and releases payloads explicitly when hiding
its native window, and pins shared window rasters during a visible exit animation.
## 内嵌图片自然尺寸

`ImageNode::natural_size_resolver` 允许结构化宿主用非阻塞查询提供已解码的显示尺寸，
保留头部尺寸作为加载前的回退。InlineFlow 的测量缓存比较实际尺寸；宿主通过既有
`TextViewState::invalidate_inline_layout` 处理方向造成的尺寸变化，保留逻辑选区并失效行高，
不重新解析或复制文档。普通 Markdown 图片不配置该查询，沿用既有尺寸策略。

## 输入框物理方向导航

`InputLine` 将原生字形簇导航接入普通文本与内联片段，原子 token 一次跨越。
Base 的左右键、Shift 选择与多光标共用该目标，密码投影及扩展字素边界只在
应用侧过滤；选区根据视觉行及端点位置折叠。行跳转复用 wrap/fold map，
按键传播比较实际目标。LastLayout 保存已有文档修订号、遮罩、token 可见性、字体/字号/方向及 Tab。
字体配置与已绘制行分开校验；普通文本的新修订也能临时塑形，拒绝借用旧字形。
已绘制行还须匹配当前 wrapper 的字体/方向、换行、缩进与 Tab，Tab 设置即时更新映射。
视口内借用有效行，其余仅生成请求软行的最终布局；需要跨行上下文时临时塑形所属硬段落。
InputLine 保存所属硬段落的已解析基方向，跨软行导航不重新探测续行方向。
绘制和离屏塑形共用硬段落方向入口；Auto 借用 Rope 分块解析，固定方向不扫描全文，
遮罩的自动方向使用中性显示文本的 LTR 基方向，避免源内容改变呈现方向。
占位文字沿用配置方向。wrapper 的配置校验和原生前缀宽度测量也包含方向；
宽度缓存以当前文档的源字节区间及已解析方向为键，只保留标量宽度，
LTR/RTL 共用一个 FIFO 和 32,768 条上限；不再复制、保留或哈希片段字符串，

普通输入的断行度量先预检完整硬段落的原生簇边界，再借用原有字素终点表建立临时源序前缀宽度；不使用双向视觉坐标之差，也不新增常驻段落缓存。Tab 只增加指向同一终点表的稀疏索引，按每个请求软行的起点重置制表位，和绘制共用 f64 位移公式。LTR、RTL 与混合文字均按实际原生视觉单元检查 Tab 分隔区间的源序，允许区间内部自由重排；每个 Tab 必须有独立簇。只有非负 advance 位于共同二进制网格、总单位数不超过 f32 的精确整数范围且与原生行宽相同时，才复用源序累计，防止重排或软行减法的舍入误差触发整格 Tab 跳变；不增加高精度逐字素副表或近似容差。其余 Tab、簇内切口和非字素缩进边界保留独立测量路径。普通 LTR 的流式定位同样读取 advance 单元，忽略字形墨迹偏移。Tab 展开显式接收已解析的硬段落方向，双向行通过 GPUI `visual_cluster_advances` 借用已有视觉单元，只收集实际 Tab 的位置，避免为此建立整个段落的源序索引。RTL 以右侧起点计算，累计位移同时用于字形、advance 和内部插入点；同源多字形只增加一次 Tab advance，并保留布局 ink 裁剪。Token 片段的测量和占位文字的导航同样显式传入所属段落方向。保留布局通过 GPUI 字体修订只读快照拒绝陈旧导航几何；断行、Token 度量与最长行宽度的缓存同时区分字体集合修订。快照仅共享现有原子计数器，不保留 TextSystem 或字体载荷。未换行输入的最长行宽度同时区分遮罩状态和段落基方向，遮罩启用时测量实际遮罩字符。
移除片段字节预算。仅宽度变化复用，文本、字体/方向、Tab、缩进和内联度量
变化均先整体清空；绝对区间由 Rope 行起点与显式测量区间组合，不反查指针，
不跨文档修订复用。字体/方向合并更新，
配置不变时不触发折叠投影重建。普通、非遮罩、无 token 的多行输入在所有
断口都位于真实字形簇边界时复用完整硬段落塑形，保留连接形式和嵌入层级，
并由同一段落游标逐行执行 L1/L2。绘制和离屏导航共用全部断口的预检；
预检使用 skip_until，不生成被丢弃的行，新增 rewind 复用原字形索引和 Unicode
分析后再生成最终行。离屏导航不保留其余软行，不增加常驻段落缓存。
簇内断口、token/遮罩段落映射及断行测量的完整邻文上下文仍未补齐，
这些情况暂保留原逐行路径，不视为完整复刻；尚未编译验证。
绘制与导航共用普通文本/token 行塑形、高亮范围裁剪和 Tab 处理，token 的
不换行宽度测量也计入 Tab；原子 token 内仍不绘制源文本的空白标记。
多行绘制借用 Rope，按需生成软行或完整硬段落的塑形输入；离屏测量只保留
请求软行的最终布局。没有 token 的行不分配内联片段数组，不增加常驻文字快照或行缓存。
跨行前释放临时行，所有头的
目标按原顺序预计算，普通单光标保存在 SmallVec 栈内，之后使用已有移动/选择生命周期。
每个 CursorSelection 统一持有双向命中侧和软行末尾归属，移除活动光标的全局软行标志。
移动、选择、新增和绘制都使用各自的侧；既有撤销快照直接保留这份状态。
编辑或合并改变头部时，两项视觉侧共用源索引戳失效；视觉侧变化参与选区比较与显露。
遮罩/IME 的绘制快照绑定显示索引，列选择的按下、拖动、自动滚动和每行端点保留软行归属。
垂直新增光标用源头的完整命中与软行侧计算锚点，不扫描其他光标来推断该位置。
左右导航和 Shift 选择的目标连同列锚点预计算，支持离屏及普通文本重绘前的新修订。
列计算复用有效已绘制几何，否则测量目标行与首行缩进；居中/右对齐需要同硬行
各软行的最大宽度，保留目标行及至多一个临时测量行，其余即时释放，不建立硬行缓存。
空文档沿用占位符首行命中、最大宽度与
既有单行/多行 Tab 差异，不把占位符作为源文本。共享选择生命周期消费显式锚点，
裁剪改变头部时不继承该目标的锚点；其他列查询也拒用旧修订或配置的字形。
上下、Page Up/Down、Shift 上下选择与垂直新增光标共用原生行几何、源列锚点和
预计算目标；覆盖离屏与普通文本重绘前的软行，取消按源字节列近似离屏命中。
行原点优先借用已绘制的最大宽度与续行缩进；冷几何与列查询共用同一规则。
垂直选择保留初始横坐标，不因经过短行而丢失；目标命中保留双向侧与软行归属。
字素/CRLF 裁剪限定在目标行。没有相邻可见行时，普通移动保留原命中，Shift 扩展
至源文档边缘；测量缺失时保留命中，不误触发文档边缘选择。
垂直新增光标消费统一目标，移除另一个源光标快照数组。
Native input movement now repairs a missing font/token measurement recipe at
this input event's effect-cycle boundary. After releasing the input entity,
it performs one ordinary GPUI draw, clears the draw arena through its normal
token, and synchronously resumes the original action through the retained
focus handle. Scoped `FocusHandle::resume_action_after_capture` reuses the
existing bubble path, without repeating global/window capture listeners. Base
registers one native movement listener per action on its focused node, as this
entry point requires. This preserves parent propagation at a physical row edge
and avoids a queue waiting for a later platform frame. Left/right, up/down,
Shift selection, page movement and vertical cursor creation use this cold path;
measured navigation stays unchanged. A weak entity, document revision, active
cursor identity and full caret/selection snapshot prevent an old action from
replaying after an edit, selection replacement or focus change. A failed single
draw logs the missing recipe and does not recurse or substitute source order.
The extra draw includes real token presenters; it does not invent token widths
or create another geometry/font cache. The change is not compiled or tested.
空源只有一个停靠点，不依赖字体/token 度量，不进入额外绘制。
原生塑形省略行首控制字符但仍保留字形时，左右导航复用绘制所用的首个逻辑簇
起侧，再沿原生簇顺序重试或判定行边缘；临时命中不替换原始源锚点。内联文本
在进入相邻片段前完成同一修复，避免跳过当前片段文字。普通移动成功时不增加
扫描，没有额外字符表、源码副本或缓存。整行没有原生字形等缺少插入点的情况
仍可能使用既有源顺序路径；完整段落重排也未完成，按词导航仍按源顺序。
这些改动尚未编译或验证，完整范围以 README 的实现状态为准。

富文本 Tab 换行在构建普通断行片段之前，直接将 `tabs::Widths` 的既有 f64
累计位置借给 GPUI；范围测量游标只借用该表和稀疏 Tab 度量。普通候选与
位置相关测宽因此共用源边界，不再为双向文本分配另一份逻辑 advance 数组。
保留原生簇、nowrap、空白策略和对象替换宽度；不复制源码、不增加常驻缓存。
这项改造尚未编译或验证。

## 滑块原生焦点与用户变更

Base `Slider::track_focus` 显式启用原生焦点、Tab 遍历与键盘调节；未提供焦点句柄的
既有调用保持原默认行为。捕获阶段只接入左键焦点，拇指仍拥有原拖动手势。
支持方向键、Page Up/Down、Home/End，步进从最小值对齐，页面步长采用区间的
十分之一与单步的较大值，再按步进取整、限位。方向与键处理依据
[WebKit 原生 range 输入实现](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebCore/html/RangeInputType.cpp)。
键盘和无障碍调整保留范围值的类型，与指针一样发出 `Change`；程序 `set_value`
不发出用户变更。键盘与无障碍的离散调整还发出 `Release`，到达边界而值未变化时
不重复通知。`accessibility_value` 可在拇指显示动画期间暴露权威目标值，默认仍使用状态值。

指针数值仍使用原先测得的内侧轨道边界，步进后重新计算拇指位置，避免显示比例与
实际值不一致；值未变化时只在交互刚开始通知一次。释放后通知状态以重绘按压反馈，
`is_interacting` 与 `cancel_interaction` 供宿主在失活、隐藏或控件移除时结束指针及
触控拖动。鼠标释放包含捕获阶段与外部释放处理，不改用轮询或另存轨道几何。
以上为源码实现说明，尚未编译或验证。

## 列表外侧标记的首行基线

结构化宿主的外侧标记不再固定于项目顶边。普通 `Inline` 从已测量的
`TextLayout` 取得首行基线，`InlineFlow` 从原有首行行框取得，避免将图片或
上标片段自身的基线误当作整个行框的基线。当前预绘制作用域只记录第一个正文
基线，嵌套项目返回其正文结果，标记不进入观测或选择。观测栈按作用域弹出，
不保留文字、行布局或文档引用，不增加常驻几何缓存。

外侧标记继续使用原标记盒的字体、字号、颜色、方向和宽度；复用标记本身的
文字测量，补偿盒内位置及元素边界吸附，保持混合行的子像素基线。透明包装器
直接返回正文的布局 ID；原生 LI 的 Root 直接作为正文，去掉额外容器，使其边框、
边距与背景成为标记的锚点。正文与标记各预绘制一次，预绘制不重测或重新塑形，
Markdown 和任务复选框保留既有路径。
正文容器显式使用原生 Block，混合行与普通文字在现有尺寸测量中一并返回首行
基线，不额外保存或测量正文。标记仍只独立测量自身一次，其实际盒高和基线经
`Window::set_outside_list_marker` 交给同一正文布局 ID。Taffy 的限定补丁在正常
块流内处理较高标记推动正文、后续块的位置与高度；没有正文基线时顶边放置，
以标记实际高度撑开空项，包括没有子节点的叶节点，取消估算字号的 min-height。
该占高不生成正文基线；空项背景仍由正文盒绘制。
精确浏览器规则参照
[Blink 的外侧标记布局](https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/layout/list/unpositioned_list_marker.cc)。
较高标记与子块自身的上升部比较，不扣除该子块的顶部位置或先前块高度。
固有高度查询需要完整子布局基线，复用 Taffy 的最终结果缓存，避免尺寸缓存和
Flex/Grid 的只求尺寸路径吞掉基线而漏算位移。约束改变仍按普通布局重算。

宿主 `editor_host::baseline_block` 保留内容布局 ID，不新增 Taffy 节点。
HTML 表格和数据详情共用透明宿主的 `auto_table_block` 入口，设置限定自动
行高、表格行基线及内容列宽策略。HTML 使用自动轨道，并由跨列单元格约束
所占列的最小和最大内容宽度；数据详情保留其首列定长偏好。
`table_block` 继续提供只启用自动行高的入口；
`baseline_block` 仍供只需要原生盒基线的普通自定义块使用。
`native_table_cell` 在原选择包装器内设置对齐元数据；格子必须是含单一正文块的
列 Flex，并伸展至完整行高。原生入口统一设置 top/middle/bottom/baseline 对齐，
不通过绘制位移移动正文，不缩小背景、边框或命中区域。二者将原生盒基线
提供给外侧标记。基线只读取现有最终布局缓存，不复制 LayoutOutput、
再测量或新增缓存。预绘制期间阻止单元格后代的首段文字代替该盒基线，
无原生基线时也保留 None；嵌套列表仍有自己的观测作用域。

补丁依赖及边界见 `taffy/PATCHES.md`。表格行基线在最终列宽下计算，
偏移参加原生排版和原有缓存键；无参与单元格时使用首行及单元格有效底部留白，
空内容不冒充文字基线，跨行上升部保留在起始行。跨行分配复用已有轨道与
单元格字段，保留实际空行；正文位移不改写 padding 或格子尺寸。

GPUI 的 `unrounded_grid_area_bounds` 借用 Taffy 已保留的详细网格轨道位置，
读取含隐式轨道的非空行列范围，返回与未取整子项访问器相同的窗口偏移坐标。
非网格或越界返回 None；不重建轨道、不复制详细布局，也不增加几何缓存。
HTML 表格用它补绘短行中未被实际单元格占用的行/节背景；跨行覆盖按真实
逻辑列计算，RTL 只镜像最终列范围，已有单元格的透明色合成不重复应用。
完整表格不保留额外行样式，也不运行空位覆盖计算。
应用预览不保留作者行/行组高度、百分比尺寸或任意边框声明；此约束来自旧项目
样式白名单，不作为表格扩展目标。特殊空内容、嵌套空块、折叠边距及 DPI
组合仍待核实。
尚未编译或验证，也未宣称列表布局完整对齐。

## 行内流的测量字形复用

Base `InlineFlow` 保留本次断行的硬行测量结果，将可复用的源范围字形传至
`Inline` 和 GPUI `StyledText`，用于最终绘制、装饰、选区与命中。整段未切分时
共享原字形 `Arc`；软行切分只复制必要的字形记录，所得几何不再按行文本重复塑形。
子串只提取一次并由最终片段共享。临时硬行和源游标随布局计算结束释放，片段几何
归现有每流至多三个宽度布局所有；源内容、样式、方向、字号、rem 或字体代际变化
仍通过原布局键失效，视图释放时沿原所有权链释放。

复用要求切口处于原生塑形簇边界。普通 Markdown/编辑器还要求片段包含硬行
原有的请求字体和实际回退字体集合，避免把缺席字体的升降部度量带入短行。
受控行内盒完整覆盖的软行由盒自身提供字体支柱与半行距，字体集合变化时
通过 `ShapedLine::with_vertical_metrics` 按实际字形字体更新局部升降部，不重新
塑形或丢失整段连接上下文；字形位置相对基线，水平位置与内部光标不变。
字体集合相同时沿用原有度量；软行已独占其布局，更新不另复制字形。
度量查询使用已有文字系统字体缓存，不增加字体或行几何缓存。完整硬段落另外可用
`ShapedLine::cursor_with_direction`：共享段落级 bidi 解析结果，按软行应用 L1/L2，
源索引只排序尚未提取的字形记录，每个字形仅复制到最终软行一次。字素内的
回退字形保持原生相对位置，内部光标随其字形簇使用相同位移，重置空白方向
同步用于命中。该游标和原生软换行共用 `CursorBidi` 的逐行解析，不另写重排规则。

完整未切分的段落直接共享原布局，不创建游标或 bidi 分析；真正切分时才按需
创建源索引。普通游标仅增加可选指针，不内联预留双向解析器的向量与状态。
跨字号或原子对象的双向段落、软行/字号连字切口及不受完整受控盒覆盖且字体集合
一般渲染调用对不一致的软行保留既有塑形路径；启用 `require_native_layout` 的富文本文档则保留失败状态并明确提示，不退回片段重塑。不改写逻辑源文本；行内方向使用上述独立表现源与原文索引映射，不再由适配层预先拒绝。
必要重塑采用原硬段落的基方向；所得 `Arc<LineLayout>` 同样交给最终文字元素，
不在绘制端重新按软行推断方向或再次塑形。行盒对齐与字形的段落方向分别保留，
中性 plaintext 行的两者可不同。
附加字距和当前高亮沿用原处理，最终保留元素同时按预排版 `Arc` 身份失效，
不新增全局片段缓存。纯绘制连字切分已接入；软行/字号连字断口仍待补齐。行内隔离已接入，完整 bidi 流布局仍待集中验收。
以上改动尚未编译或验证。

## 行内流的固有最小宽度

可换行流的 `MinContent` 请求以极窄断行求最宽不可分片段，不能沿用不换行
宽度；显式 nowrap 仍使用不换行路径。继续使用原有字素安全断行、不可换行
片段、字体及字形复用。零宽度一次消费原生断行迭代器，避免逐行重建剩余
源片段；固有探测布局不进入三个显示宽度布局的缓存，数值尺寸交给 Taffy
原有测量缓存。正常最终宽度仍保留完整绘制、选区和命中几何。
此路径尚未编译或验证，复杂方向与折行组合仍须集中核实。

## Select 的原生固有尺寸与触发器定制

`Select::intrinsic_size(true)` 取消组件两层包装的强制满尺寸，保留调用方
对触发器的尺寸约束，由原生布局按当前选项计算宽度。默认仍保持原有满尺寸。
`Select::trigger` 提供已经绑定点击行为的根、选中标题和清除按钮/图标，
以及现有实体的悬停、焦点、禁用和展开状态；应用可以设置外观及动画，
不另建选择状态、焦点实体或文本测宽缓存；动画沿用应用现有的 Kit
颜色通道和减少动态效果设置。定制入口保留调用方图标的
尺寸和颜色；无外观的定制标题通过根继承颜色和禁用透明度，默认标题
仍沿用 Kit 的禁用色。外层 BaseSelect、列表、清除、键盘、可访问名称
与弹层定位逻辑保留默认行为；菜单的额外定制见下文。

## 可编辑组合框的弹层可访问属性

`Input::aria_expanded` 与 `Input::aria_has_popup` 通过 `InputAccessibility`
投影到实际可编辑节点，避免把组合框的展开状态只放在装饰框或图标按钮。
字段默认均为 `None`，不修改普通输入的角色、Tab 顺序、编辑和 IME 引擎。
日期输入在应用层复用标准输入、Base 按钮、PopoverState 和 Positioner，
其草稿与受限日历状态不进入通用编辑器。不新增一套文本编辑或焦点引擎。

## Select 菜单的定制与原生流式选项

`Select::menu_style` 可提供 `SelectMenuStyle`：面板样式、行渲染器、
是否使用流式行、进场动画开关、非动画面板间距以及原生滚动条样式。
未设置时保留默认虚拟列表与弹层动画。`SearchableListRow` 将活动项、
已确认值和禁用状态分别交给渲染器，并保留已有行内容；外层 List 继续
持有点击确认、索引与滚动行为。适配器只给可用行绑定悬停游标更新。
定制 Select 的标题取已确认项快照，避免鼠标悬停或键盘游标改变当前值。

`List::variable_rows(true)` 面向需换行的菜单，使用原生流式子节点与
`ScrollHandle` 的实际行高，保留逻辑节/项索引，不采样统一高度，也不
生成虚拟列表的尺寸数组。每个逻辑条目保留直接子节点，滚动定位因此
不需要额外的行高模型。此模式渲染全部菜单条目，不用于大规模历史列表。
滚轮在滚动容器外层停止传播，避免带动承载页面；重新打开时重置偏移，
随后以最近边缘显示活动项。禁用已展开的 Select 会清理查询并关闭。

流式菜单使用 ListBox/ListBoxOption；新增可选的 delegate 名称与选中态
接口，选项名称复用 item title，选中态来自选择快照，活动项独立标记。
接口均有默认值，不改变原有 List 的角色。应用共享细滚动条样式，
不为菜单另建页面缓存或尺寸缓存。

`SelectMenuStyle::trigger_navigation` 为非搜索菜单保留触发器焦点，
支持跳过禁用项的循环方向导航、Home/End、Enter/Space 确认与打开、
Escape 返回焦点和 Tab 正常遍历。700 毫秒前缀通过单调时钟判断，
无需重启定时任务；打开时从活动项后循环查找，关闭时直接匹配并走
既有 List 确认通道。确认按 delegate 的实际索引范围和可用性校验，
不再依赖已准备布局缓存，因而首次关闭状态的文字选择也有效。
列表可用性同时约束点击和确认；Base 导出已有 `A11yDisabledExt`，
在同一个原生节点提供禁用语义，不增加布局或焦点节点。装饰性勾号
从可访问树隐藏，面板与触发器共享已有的可访问名称。

Base Select 增加可选的原生捕获键处理入口及 Listbox 弹层语义。
触发器导航启用时先处理上述按键，默认 Select 仍走既有动作绑定。
重复点击展开的触发器可关闭；点击外部关闭不抢回焦点。窗口激活与
可见性监听仅在该模式展开期间持有，关闭、隐藏或状态释放时回收。
选项更新保留仍可用的活动位置，否则选首个可用项，无可用项时关闭。

`viewport_fit` 用可用视口高度替代固定菜单限高。Base Positioner 的
`dropdown_space` 与 `dropdown_flip` 共用垂直空间计算，保留 8px 边距，
下方空间小于内容高度和 180px 阈值且上方更宽裕时才向上翻转。
预先只允许候选方向的最大高度，布局后再比较真实面板高度，避免截断
后的高度导致反复翻转；列表限高从该空间扣除面板实际边框。
面板宽度按原生视口约束，锚点离屏或无可用空间时关闭。

Select 面板通过 Core Deferred 的可选延迟布局读取触发器本帧的边界，
默认的 Deferred 布局路径不变。菜单仍走同一套原生布局、绘制与定位，
不维护独立的测量高度或持久面板几何。RowsCache 在节数量和行数未变
时不分配节向量；测量尺寸变化只更新统一尺寸数组，复用逻辑索引；
流式模式直接复用已有空尺寸数组，不创建无用的逐行尺寸。

触发器导航菜单仅在展开期间订阅 `Context::observe_window_scroll`。
原生容器的实际偏移变化才通知；通过 `ScrollOffsetEvent::is_from` 与
现有列表的 ScrollHandle 比较，忽略菜单自身的滚动。外部容器变化在
子树预绘制前关闭菜单，使随后延迟布局直接跳过已关闭的面板。
不把不可滚动区域的滚轮输入当成滚动，也不在应用层轮询页面偏移。


### 下拉菜单的自动滚动条占位

`SelectMenuStyle::scrollbar_gutter` / `List::flow_scrollbar_gutter` 显式启用 GPUI 原生自动 gutter；默认关闭。应用的下拉菜单设置 6px，与细滚动条样式共用同一宽度。流式行按扣除实际 gutter 的宽度换行，未溢出时保留完整内容宽度。

原生 `Scrollbar::scroll_viewport_from_handle` 可与既有 `viewport_from_layout` 配合：轨道在父容器左上角定位并使用包含 gutter 的外框，避免受前一个滚动子节点的块级静态位置影响；滑块比例和滚动范围使用 handle 提供的有效视口，无需应用保存几何副本。显式的 `scroll_viewport_size` 覆盖仍优先。`Always` 模式不安排只用于淡出可见性的 idle 任务。


### 保留触发器焦点的弹层无障碍语义

`BaseSelect::accessibility_controls` 独立于 `content_focus_handle`，仅展开时声明受控列表；`List::active_descendant_owner` 将高亮选项的语义焦点关联到控制触发器。应用的原生下拉模式同时设置这两者，因此列表在 deferred 层绘制时也能报告当前活动选项，并保持键盘焦点在触发器。列表未出现或已关闭时，本帧不会留下失效 controls。未配置这些选项的 Kit 控件维持既有语义路径。


### 按词移动和删除的词典边界

GPUI `LineWrapper`、原生行布局及 Base 输入的测量断行共用公开的 `line_break_indices`。
它复用锁文件中的 ICU 2.4.0 编译词典，保留 GPUI 原有默认 line-rule 选项，
将不支持复杂文字的构造器替换为 dictionary 构造器，为泰语、老挝语、高棉语
和缅甸语提供必要的词典上下文。输入继续过滤非字素边界；独立的逐字符
搜索对照入口共用同一合法断口策略，不改变其搜索算法或删除断言。
不增加依赖、模型下载、常驻断口索引或另一套分词缓存；具体语言/标点规则
与原页面的实际呈现仍待集中核实，尚未编译或验证。


`InputBaseState` 的左右按词移动、共用删除及双击选词共用一条 Rope 词范围路径，通过 GPUI `word_ranges` 使用 ICU 编译词典。普通可选文本和富文本段内选词也使用同一字节边界，移除两份手工 Latin/Cyrillic 字符分类和扫描。结果继续经过既有字素、选择、token 归一化和撤销路径；保留密码整段词语义、尾端亲和、硬换行及各控件的 EOF 命中策略。

命中 ASCII 字母、数字或下划线时，Rope 与普通字符串只取最近空白之间的上下文并包含两侧分隔符，边界仍由同一 ICU 迭代器判定。复杂文字、组合标记、格式字符和空白命中保留整条逻辑行上下文，避免截断词典词、忽略字符或空白组。单叶切片和普通字符串借用文本，多叶上下文仅在当前操作期间拼接，不保留分词索引缓存。无效 UTF-8 中间偏移只向前回退至字符边界，不扫描整个前缀。


## 行内流的分段源文本词选择

`InlineFlow` 为当前注册建立一个由所有片段共享的不可变分段源快照。文本沿用 `SharedString` 的共享存储及短字符串值语义，图片和自定义对象使用 U+FFFC 占位，不持有图片资源或渲染器。常见单项来源使用内联小数组，只有实际存在多个源项时才分配源引用数组；无文本源项的流不创建该快照。快照随现有 `TextSelectionRun` 注册和回收，不建立全文、词边界或额外几何缓存。

片段登记源项索引及其 UTF-8 偏移。窗口选择层按源顺序取得光标两侧上下文，跨样式文本项使用同一 ICU 词典边界，保留硬换行、忽略字符、空白和对象边界；软折行不截断分词。单个非空源片段借用切片，只有上下文实际跨多个非空源片段时，才按精确字节容量拼接当前上下文，操作结束即释放。结果仍映射回源项索引和 UTF-8 偏移，不建立全段前缀索引，不转换 UTF-16，也不从可能省略折叠空白的绘制行重组源文。

双击、整词拖动和 Shift 扩展共用此入口，按上游/下游亲和在现有片段布局中定位两端；行盒高度、绘制、复制及逻辑选择继续走既有投影。找不到精确端点时不推测坐标。交给窗口选择层前，清理旧的视图局部覆盖和离屏 AST 选择，避免复制混入上一次选区；触摸双击策略和三击行选择保持原有语义。


分段源属于 UI 线程，使用 `Rc` 共享，避免逐片段原子引用计数。图片与自定义行内对象的复制投影同时登记分段源的弱身份和源项索引；当词端点落在对象上，直接查询同帧既有复制投影的精确左右边界，不新增对象几何数组、映射或缓存。源快照只弱引用投影所在的帧，原子对象投影也只弱引用源身份，不持有选择参与者，从而避免窗口注册、源快照和复制投影互相保活。跨帧匹配源身份，已回收或未登记的端点不使用旧坐标。


## 图片的原子选区与源复制

行内图片复用 `InlineObject` 的同帧复制投影与行盒边界，不伪造文字布局，也不另外保存几何表。图片包装只接收已测量尺寸和原生元素；普通自定义对象在提取元素后同样不再保留多余的测量内容所有者。图片的选择色在原生图像绘制之后覆盖，避免被不透明像素遮住。图片链接保留原生点击和辅助点击识别，并在渲染时捕获所属视图，以便事件发生时检查实际选区；不依赖事件期间已退出的全局渲染栈。只读图片双击触摸策略沿用文本，鼠标双击和三击由窗口选择层分别解析对象与所属行的选择单元。

图片和自定义对象共享 `Option<bool>` 的原子选择状态：`None` 表示本次程序化范围尚无原生投影，`Some(false)` 明确排除，`Some(true)` 明确选中。段落清理重置为 `None`；同帧复制投影写入明确结果。保留程序化选区的绘制不把 `None` 改成未选中，不可选视图不改写原子选择状态。源范围和 Markdown 重建以明确的图片投影为准，仅未投影图片沿用文本运行边界的推断，避免相邻未选中图片被错误导出。Markdown 候选只借用节点，选中后直接生成最终输出片段，省去候选标记序列化与中间拼接。

托管图片不可见时仍报告原可见性回调并登记原子复制投影，但内容使用无图片资源的空元素，不创建 `img` 或触发图片解码；实际行盒来自既有流布局。弱源身份和弱帧引用的生命周期约束保持一致。

## 行内原子对象的窗口选择单元

`TextSelectionHandle::resolve_atomic_unit_with` 为富文本参与者安装可选的纯几何查询：接收窗口坐标和多击次数，双击返回对象的两个停靠点，三击及以上返回所属行的两个停靠点。窗口的初次选中、整词/整行拖动和 Shift 扩展共用该入口，再回到普通文本布局查询。未提供查询的参与者保持原路径。查询在窗口选择借用期间执行，不得排版、更新实体或发起资源工作。

适配器仅弱引用既有绘制帧，从 `InlineObjectSelectionProjection` 查询当前仍有效的对象边界与所属行边界，不新增几何数组或索引。对象复制投影携带由流布局传入的行边界；纯文本帧用同帧布尔标记跳过遍历，标记在登记、换帧和撤销复制投影时一并维护。未命中、零尺寸或已失去源状态所有者的投影不提供选择单元，也不回退到虚构坐标。

原子包装的鼠标多击清理旧视图覆盖和离屏 AST 选择，然后让窗口控制器建立真实范围，不再保存局部词字符串及其重复端点。既有复制回调按当前窗口范围刷新同帧投影，不依赖多击后的首帧绘制。选中对象与文字片段共用触摸手柄端点登记；图片的触摸双击继续抑制选词，长按沿用窗口选择机制。原生子控件的无障碍角色与事件继续由子元素提供。

## 受控文档的行内盒与背景层

`editor_host::InlineBox` 表示有身份的不可变行内盒，以 `Arc` 共享祖先链，保留本元素的计算字体、字号、绝对行高、相对于宿主当前正文 em 的累计基线位移和可选底色。透明盒也参与排版；相同颜色或字体的兄弟元素保持不同身份。`TextMark::inline_box` 与 `ImageNode::inline_box` 分别携带文字/原子标记与图片的祖先链，`ImageNode::background` 是图片自身的替换元素盒底色。`InlineBox::root` 描述段落自身的计算字体支柱，包括默认正文、标题、表题和单元格；根不绘制行内底色。`root_reusing` 仅在字体族、字重、字体样式、字号和行高完全相同时复用既有祖先根，不把祖先行内盒的底色或基线位移带入新块，也不增加全局缓存。接口不替代 Markdown 原有 highlight/code 样式。

行内盒度量归入已有内容、排版和字体代次键，与现有有界宽度布局共享，不建立全局字体缓存。该表按原生基础字体、字体代次、不可变盒身份/顺序和零宽锚点位置失效；图片几何到达或自定义对象重新测量只失效定位，不重建不变的盒度量。每行仅激活实际出现的盒及其祖先：有自身字形的盒按原生塑形中实际使用的字体分别计算半行距，没有自身字形的盒使用其主字体支柱。段落根支柱始终参与计算；图片和自定义对象继续提供自身几何约束。字体 descent 使用字体坐标的负值约定，塑形行的 descent 仍为正值。字体内容高度与行距分别计算，背景使用字体内容盒，不把半行距计入底色高度；依据 [CSS 2.2 行高规则](https://www.w3.org/TR/CSS22/visudet.html#line-height)。

`InlineNode::empty_inline_box` 保留非替换空元素的盒。流内 `Box` 项只携带盒身份与绝对流坐标，长度为零；行内流、空盒锚点、选词与编辑器统一使用 UTF-8 源坐标；图片和自定义对象均占一个 U+FFFC 的实际字节范围，`ATOM_LEN` 从同一静态源字符取得。空行位置直接交给既有编辑器接缝，不再逐行重扫对象并转换单字节占位偏移。锚点随同一度量表排序、按行定位，在软换行边界归上游行，不插入文本、占位字符或单独塑形片段。纯文本段落沿用既有文本/标记缓存，锚点不切断相邻文字的连字；硬换行没有字形片段，但其行内盒参与被终止行的度量。没有文本、保留空白、替换对象或强制换行的行不占高度；空盒仅参与实际存在行的计算，末尾换行不额外制造一个光标空行。受控白名单没有行内非零边距、边框或内边距，这些不构成该判断的输入。参见 [CSS 2.2 空行盒规则](https://www.w3.org/TR/CSS22/visuren.html#inline-formatting)。普通 Markdown/编辑器的既有空行行为不受此规则影响。

行内流预绘制为文字、图片和自定义对象传入既有行布局的共同基线。文字用原生 advance 单元按源范围登记，排除专供选择使用的换行标记；对象用已有盒宽登记。背景顺序及字体几何复用上述度量表，祖先先于后代，兄弟保持源顺序。内容绘制前，同一源层同一行的相接矩形合并后提交原生 quad，避免拆分片段重叠混色；图片自身底色在图片内容包装内覆盖完整图片盒。

可见背景矩形随预绘制状态回收，不持有图像资源。文字的字形单元在同帧范围和选区绘制中复用；没有带底色祖先链的片段不采集这些单元。原子对象边界的两端对齐间距归入相邻文字的前侧或后侧字簇单元，背景直接读取调整后的原生单元；对象保留固有宽度，不另存脱离内容盒的片段间隙。



## Inline vertical alignment

`editor_host::InlineVerticalAlign` and `InlineBox::aligned` describe
`middle`, `text-top`, `text-bottom`, `top` and `bottom` without changing
source text. Only font-relative values require an explicit parent
typography scope. Anonymous paragraph parents use an ordinary font scope,
not another root strut; aligned siblings cannot create multiple roots.

The flow-owned font table resolves primary-font offsets and each scope's
nearest line-edge ancestor once. Each row collects actual fallback metrics
before resolving parents and children, including empty scopes. Primary-font
rows may share one immutable geometry; fallback differences and edge-group
translations retain sorted sparse overrides. Ordinary rows allocate no
geometry. These projections belong to the existing bounded flow layouts and
font-generation key, without an independent global cache. Text segmentation
also distinguishes the closest aligned ancestor; ordinary descendants within
that scope may continue shaping together.

Line-edge roots start with a local zero baseline. The row accumulates their
own boxes and non-edge descendants, including scoped images/custom objects,
into aligned-subtree bounds. Nested top/bottom roots form independent groups.
The normal paragraph strut and all group heights determine the minimum line
height; every member of a group receives the same final translation. The
existing deterministic bottom-then-top constraint policy for atomic images
also applies to text groups, independent of source order. A top/bottom image
inside a group remains its own line-edge constraint. Group membership uses
the flow-owned table instead of repeating ancestor walks. No temporary
document, source-text copy or extra shaping is introduced; ordinary rows
skip group translation and rows without scoped atoms skip atomic collection.

Text paint, selection/hit geometry, inline fills and atomic fragments share
the resolved offsets. `DecorationOrigin::inline_box` anchors propagated
strokes to their source scope, so shifting a descendant does not move an
ancestor's underline. Block formatting boundaries clear/rebase the scope.
Scoped atomic extents contribute after font-relative resolution and before
final group placement. Each fragment receives its final displacement once.
The contract follows [CSS 2.2 aligned subtrees](https://www.w3.org/TR/CSS22/visudet.html#propdef-vertical-align).

## Source-preserving white-space policies

`TextMark::space_policy` and `InlineHighlight` carry optional GPUI
`TextSpacePolicy` independently of `no_wrap`. Normal/nowrap/pre-line can
collapse edge spaces without conflating them with pre's preserved advances;
pre-wrap and break-spaces retain distinct wrapping behavior. Highlight merge,
flow equality, native paragraph ranges and suffix fragments retain the policy.
Adjacent equal native ranges coalesce without splitting font or bidi context.
Native fragments use the existing range slicer for all mark fields.

Row spacing now processes explicitly authored space policies independently of
justification. Collapsible prefixes/suffixes lose advance through the existing
`ClusterSpacing` geometry; source bytes remain unchanged. Pre-wrap suffixes
retain glyph, decoration and selection advances but hang outside the row's
fit/alignment width. Forced/final rows hang only their overflowing portion,
resolved before justification. Paragraph RTL places that retained suffix on
the physical left using the same row offset as paint and hit testing.
Break-spaces never trims or hangs its preserved separators. The contract follows
[CSS Text line-edge processing](https://www.w3.org/TR/css-text-3/#white-space-phase-2).

Unjustified rows only retain edge clusters and one interior source barrier;
fragments without edge white space skip native glyph collection. Small cluster
sets stay inline, grapheme validation streams boundaries, and highlight lookup
advances in source order. Justified soft rows retain the full opportunity set.
No paragraph cache or normalized text copy is added. Native clusters crossing
different space policies are kept intact rather than partially shortened.
These changes are uncompiled/unverified. A configured `TextStyle::tab_size`
now also measures wrapped native paragraphs from each soft row's own origin.
The Unicode/grapheme/protected-cluster iterator is reused with a source-range
measurement callback; carried words are measured again at the new origin.
Only paragraphs containing tabs allocate a temporary cumulative native-advance
index. Forward measurements stream that index and a sparse tab list; changing
the row origin repositions the cursors. No source replacement, normalized copy,
independent shaping pass or persistent width cache is added. A shared forward
merge inserts measured host-object widths into native source advances, including
objects whose placeholder glyph was omitted by the backend. Wrapping and paint
use this same merge without per-glyph object searches. Soft rows retain the hard paragraph's direction
when expanding native tab cells before paint-context distribution.

Ordinary text details, their raw view and code details use four-space stops,
matching the reference `TextPreview.vue`. Unconfigured views keep their existing path.
Preserved rich-text tabs use eight-space stops and resolve each tab's requested
font and actual shaped size at layout time. Equal used geometry coalesces into
source intervals, even across text without tabs; ordinary same-font paragraphs
do not retain a metric pair per occurrence. The wrapper and prepared paint rows
borrow the same `TabStopRun` metadata, with the existing font-generation key
owning invalidation. No adapter-time font resolution or additional cache is added.
Cluster-internal soft-wrap partitioning and
remaining unsupported white-space combinations remain separate fidelity gaps.

## Clipboard whitespace and block separators

TextView copy no longer trims selected content, and window selection only skips
empty strings rather than whitespace-only selections. `PlainTextCopy` streams
source content separately from pending structural block endings. Structural
breaks flush between copied pieces, while actual spaces, tabs and newlines stay
in the source, including at either end. Plain select-all and partial selection
share this path; custom table/file groups keep their interior separators and
empty cells without copying unselected suffixes. Markdown reconstruction keeps
its existing contract.

The writer reuses its first owned source string, and window copy hands through
the first participant's result before appending any further participants.
`CopyContent` chooses a callback or projected text, so callback-backed views do
not clone an unused fallback. No persistent selection mirror or cache is added.

`PlainCopyRange` scans source leaves and shares a scalar cursor across nested
containers and custom cell groups. Only source endpoints use partial selections;
interior paragraphs, code blocks and cells copy whole even without paint-time
ranges. Explicit virtual block coverage retains whole endpoints, and observed
empty endpoints still suppress their own content. The scan does not build a
per-node index, flattened source or persistent cache.
These changes have not been compiled or verified.


## Native caret projection and whole-participant coverage

`text_selection::project_selection_range` is the shared native-caret projection
for ordinary selectable runs, rich-text paint and synchronous rich-text Copy.
It replaces the remaining ordinary-text character-position scan, retains hit
affinity and snaps fallback-internal grapheme cuts with on-demand Unicode
context. Existing rich-text selection adapters and their tests use the same
projection; no persistent character geometry or boundary table is added.

Full participant coverage projects ordinary runs without requiring window
coordinates. Rich-text Copy uses the same whole-document Plain/Source serializer
as local Select All, so unpainted virtual blocks and prior selection overrides
cannot truncate it. Copy still rejects a pending document replacement, and
cell-confined gestures keep their existing restriction. Text, inline objects
and custom blocks derive full-selection paint from the coordinator snapshot,
rather than storing another selection flag. Touch Select All publishes Full
coverage instead of limiting the source range to registered visible runs.

Ordinary selectable participants project FromStart/ToEnd from their own
endpoint and logical run order. Only the endpoint run needs a native caret hit;
other runs contribute their whole source or nothing. Empty runs remain valid
source boundaries, and a length-mismatched endpoint layout is rejected. No
foreign participant coordinate is substituted for a logical document edge.

Snapshot invalidation still clears the painted copy cache. Copy arriving before
repaint projects retained native layouts synchronously, rather than returning a
whole-source fallback for a partial range. Paint-change detection reads only the
existing cache and therefore still observes the pending projection. The shared
copy writer sorts borrowed source ranges and appends directly into one result;
the single-run path does not allocate a sort vector or per-run strings. The
existing snapshot-invalidation case now expects the current range before paint;
its coverage and remaining assertions are retained.

Rich-text prefix/suffix projection across participants, detailed object endpoints
and offscreen touch endpoints remain separate gaps. These changes have not been
compiled or tested.


## Logical touch endpoints and unit dragging

Participants without a document-source adapter use native leading/trailing carets with explicit
Downstream/Upstream affinity. It retains their x coordinates instead of adding
physical one-pixel insets. Length-mismatched endpoint layouts are rejected.
`SelectionEndpoint::at_registered_caret` binds the existing participant and
content-key resolver directly, so an exact caret at an excluded hitbox edge does
not require a shifted hit test or select an adjacent participant. Handle drag
initialization uses the participants which reported the painted source edges.
Touch Select All resolves its queued content keys immediately after the state
update, matching the other endpoint-producing input paths.

Handle swapping compares participant document order, then source-run order and
native UTF-8 insertion points within a participant. Equal or unresolved atomic
positions preserve the current handle side. Word/paragraph-unit extension
prefers this same source ordering; physical triple-click rows and atomic units
retain their existing geometry policy. Ordinary multi-click unit carets request
the same leading/trailing affinities already used by segmented word sources.
The open-ended plain projection and endpoint comparison share the existing run
lookup and grapheme snapping; no source mirror or persistent index is added.

First internal endpoints without native layout, table/plugin handle
geometry, unresolved custom-object interiors and retained affinity at ambiguous
bidi junctions remain separate gaps. These changes have not been compiled or tested.

Touch edge aggregation, handle painting and drag ownership now share the
window snapshot's logical participant predicate, including its active scope
and single-participant restriction. Retained edges from an excluded participant
cannot become the current start or end. Native handle callbacks identify their
participant and recheck the current owner, viewport and handle hit bounds before
starting a drag or consuming the event. A stale hitbox does not start a gesture
on a replacement selection. The public programmatic drag API keeps its existing
signature; content-key resolution runs only after a drag actually starts. No
selection mirror or additional geometry cache is introduced. This change has not
been compiled or tested and does not resolve the offscreen endpoint gap above.

Ordinary handle drags now continue the source-ordered logical anchor/cursor
instead of rebuilding them from fragment carets. Preparing that continuation
clears unrelated participants and local selection flags, while retaining the
current owners' source endpoint state. The rich ordering callback weakly borrows
the current source tree and reuses cached endpoint identities; one source walk
validates leaf order and byte offsets within a block, including an offscreen
stationary endpoint. Unresolved custom-object interiors do not become insertion
points, and a known source endpoint never falls back to its obsolete coordinate
when its counterpart is unresolved. Document replacement updates the weak tree
and validates retained endpoints through the existing invalidation path.

The copy callback checks the participant's current local-selection marker before
using view-local overrides. A separate window-range serializer shares the
existing document copier, so synchronous Copy during a continuing gesture cannot
return stale word/Select All text while notifications are pending. Selecting all
locally retires the old window gesture before setting its new local state. No
additional document, source index or geometry cache is retained. These changes
have not been compiled or tested; table/plugin handle geometry remains incomplete.

Ordinary rich source ranges now report each endpoint's geometry independently.
The adapter resolves an existing source identity/offset/affinity only against
that source's retained native rows, including alignment and the native side at
soft-wrap and bidi boundaries. Atomic inline objects share their existing native
directional caret rule with word selection. Missing/offscreen endpoints report
no box instead of borrowing a selected fragment's cut. Source ordering retains
one derived value for the current logical endpoint pair, invalidated alongside
the existing projection and source tree, so paint and repeated pair comparisons
do not rescan the whole block.

Registration can explicitly report a missing start or end. Window edge ownership
is established before checking for a box, preventing an interior participant
from taking over a hidden endpoint. Each visible handle remains independently
paintable and draggable. The touch snapshot marks missing transport boxes hidden
and retains the source range's nonempty semantics; the menu anchors only to
visible edges. The public complete-pair registration/accessor and input caret
defaults remain compatible. No new glyph table, geometry cache or shaping pass
is introduced. Table-cell mode, plugin-block caret metrics and
first internal endpoints without native layout remain gaps; this change has not
been compiled or tested.

Source-aware touch Select All now asks the existing weak document for first/last
selectable block identities and captures their source boundaries in the existing
endpoint state. A boundary is explicitly marked as logical, with no window point
projection; it never resolves to a fabricated participant-origin caret. Whole
coverage still copies the document through its existing serializer. Native text
and inline-object geometry is reported independently only when each real source
edge is painted. Leaving Full coverage retains those same source endpoints and
uses the ordinary continuation path, so a hidden held end does not need geometry.
Shift extension also retains that source anchor and its existing owner state.

Source boundary lookup ignores zero-byte text leaves and entirely empty blocks.
They contribute neither copied characters nor native caret geometry, so they
must not displace a visible text or inline-object edge. This also lets the
existing document block resolver skip empty leading/trailing paragraphs. The
lookup borrows canonical chunks/state; it neither materializes text nor creates
a layout or cache. The application's table and file blocks expose structured
groups, using ordinary cross-source selection; they do not use Markdown's cell
confinement or opaque plugin-block caret paths. This change has not been
compiled or tested.

Snapshots retain each available window point independently; the public complete
pair accessor remains compatible. A different participant can therefore project
its own moving endpoint without the hidden rich boundary's geometry. Before a
moving endpoint's deferred content-key resolver runs, ordering can resolve only
an actually painted source under that point from the retained frame. It does not
invoke application key callbacks inside the window-state lease or borrow a
neighbour for unpainted padding.

Compatible document changes refresh source boundaries and retarget changed block
keys only while touch Full coverage remains active. Once it becomes a bounded
range, retained source offsets stay fixed; a missing/replaced held source is not
reattached to the new document end. Gesture initialization also refreshes Full
keys before freezing them, covering a document change since the last paint.
Retargeting or reversing the two ends carries the corresponding already-prepared
source position through the existing endpoint state. The boundary callback borrows the existing
document through a weak reference; no additional source/geometry cache or layout
pass is introduced. Table-cell mode, plugin caret metrics and unresolved internal
positions remain incomplete. This patch has not been compiled or tested.

## Source block coverage for participant prefixes and suffixes

Visible padding inside a mixed-source block can now resolve to that block's
nearest visible native row. The existing per-frame top-level box record also
retains its intersection with the active content mask; only a point inside
that intersection may use this path, and candidates must intersect it too.
The native hit query is clamped to the candidate row, while the logical
endpoint keeps its document coordinate and captures the canonical text/object
source position through the existing endpoint state. Actual native hits retain
priority over block ownership, including overflowing content. Both early key
resolution used by ordering and the deferred content-key resolver recognize the
same visible block ownership. Hidden positions and blocks without visible
native projections gain no neighbouring-source fallback. No new layout,
source mirror, per-character index or separate geometry cache is added;
nonselectable cards still skip block recording. Not compiled or tested.

After painting each block, its recorded projections determine whether it has
visible native text/objects. Only those blocks contribute their visible bounds
to the existing text hit list; empty or opaque-only boxes retain outer-edge
bounds but no native padding hit area. Each block scans only the projections
produced while painting it. The window uses the existing source-order resolver
for same-participant selection nonemptiness when it resolves successfully,
preserving a range whose two sampled pointer positions both lie in padding.
Plain participants retain their run-based path, and an unresolved source range
retains the existing fallback. Not compiled or tested.

The rich copy callback passes the current participant coverage through the
TextView serializer. A prefix has only its final block partially selected;
a suffix has only its first block partially selected. Complete blocks use
source content even when retained paint is stale or absent. A missing local
range in the sole partial endpoint does not imply selecting its whole block.
Bounded/local selection retains the existing observed-block and atomic-block
policy; a cleared bounded range still produces no clipboard content.

Open-ended copies require a resolved block range. Reversed or out-of-document
ranges are rejected instead of clamping stale content keys into another block.
Cell-confined copies require the originating block and use bounded coverage.
Window prefix/suffix copies bypass an old local multi-click text override.

The serializer walks source blocks without a document-wide selection bitmap,
and appends Markdown blocks into a single output instead of retaining a vector
of temporary output strings. It adds no persistent selection state or cache.
Native rich endpoint projection, logical object endpoints and offscreen
endpoint geometry remain pending. These changes have not been compiled or tested.

## Native rich prefix/suffix projection in source order

Rich source hit testing uses each text fragment's existing full flow-row
height while retaining its native horizontal extent. A larger font or inline
image therefore does not leave unresolvable vertical gaps around smaller
text, and adjacent source fragments remain distinguishable. Native insertion
points and hit affinity still come from the retained original layout. Padding
inside a block but outside a row resolves to a known source boundary only if
the native fragment covers its entire canonical text and that text is the
block's sole source.
Partial virtual fragments and mixed-source interior padding retain the pending
policy; no nearby fragment substitutes for missing insertion geometry.

Selectable top-level blocks also retain their actual forwarded layout bounds
once in the existing painted frame. They resolve already owned, unhit points
before/after a mixed block to the source tree's own first/last positions, even
if a child has not painted. Real native insertion points take
precedence, including overflowing text. Source identities are weak references;
byte offsets are fixed at capture, rather than following a later append.
Metadata-derived text edges pass through the original source-chunk validation
and canonical text preparation, while native hit pairs retain their fast path.
Custom blocks explicitly observe same-side empty endpoints so virtualized copy
cannot treat them as unobserved whole blocks. Bounds are cleared with each
frame and retired with document geometry; nonselectable cards do not accumulate
them. No extra layout, document copy or independent geometry cache is added.
These changes have not been compiled or verified.

Top-level document blocks forward their child's layout without creating another
layout box. During paint they scope source ownership and compute selection
coverage once for their existing text/object projections. Both normal flow and
virtualized lists use this scope. Nonvirtual content keys resolve against those
same retained projections; virtual content retains its list-based locator.

Open-ended projection resolves only the owning content endpoint, using the
retained frame's content origin (including its own scroll offset). It finds a
native fragment within that source block, resolves its grapheme-safe UTF-8
insertion point, and covers earlier/later fragments by source order. It neither
borrows another participant's window endpoint nor hit-tests a different block
when a virtual endpoint is absent. All shared text states are reset before
merging ranges; an excluded fragment cannot erase an earlier selected fragment.
Cell-confined selection retains the bounded projection policy.

The existing virtual-range state records whether the current logical range
has been projected. Endpoint/coverage changes invalidate it; a held range is
not re-hit-tested because geometry reflowed or a gesture ended. Paint and
synchronous Copy share this projection. A newly painted endpoint can resolve
after registration and request one deferred follow-up draw. No flat document
source, per-character geometry or additional layout cache is retained.

Prepared native atom fragments carry their resolved bidi direction into image
and inline-object projections. Source-leading/trailing carets, word-source
object carets, open-ended atomic selection and painted touch edges use that
direction. The word-unit geometry fallback respects a same-row reversed
source-caret pair; physical triple-click rows retain their existing policy.

First-time offscreen endpoint resolution, complete coordinator-level object
source ordering, retained affinity at ambiguous bidi junctions and contextual
soft wrapping remain gaps. These changes have not been compiled or tested.

## Source ordering across inline objects

The coordinator now accepts an internal, read-only endpoint comparison lookup
from a participant. Rich TextView supplies it through a weak reference to its
existing retained frame. The callback receives participant-relative endpoint
snapshots; it must not update a view, lay out content or enter the window state.
Ordinary text participants retain their native text-run comparison path.

The rich lookup interprets only its own block keys. Different known blocks
compare directly, including when offscreen. Within a block, the existing
source-ordered copy projections resolve the closest native insertion point;
text uses the shared grapheme-safe byte hit test and inline objects use the
same resolved-direction start/end rule as open-ended projection. Word-unit
extension and touch-handle reversal can therefore compare text with images
and other inline objects without an extra source or geometry cache.

Projection, nonvirtual block lookup and comparison share one valid-fragment
search. A known endpoint block cannot fall back to another block. The lookup
returns unknown after geometry invalidation, for a missing same-block layout,
or for a custom atomic block without native insertion points; it does not
substitute a nearby text run or estimate a custom block's logical position.
Custom-block endpoints, first-time offscreen resolution, persistent bidi hit
affinity and contextual soft wrapping remain gaps. No compilation or tests
have been run for these changes.

## Bounded rich selection through source projections

TextView now resolves both owned endpoints before updating a bounded range.
The retained native projections provide source-block order, fragment order
and grapheme-safe byte offsets; reverse gestures normalize that source pair.
All represented shared text owners are reset before slices merge. Inline
objects use the same resolved-direction boundaries as endpoint comparison;
custom blocks inside the range remain atomic. Known interior top-level blocks
paint whole and out-of-range blocks paint excluded.

The existing projection stamp now covers bounded and open-ended ranges. Paint
and synchronous Copy share it, held ranges survive geometry reflow, and the
first frame with both native endpoints can request one deferred redraw. Cell
confinement, custom atomic endpoints and endpoints absent from the retained
frame preserve the existing bounded projection policy. Failed native endpoint
resolution makes no partial source mutation.

Resolved zero-byte endpoints are explicitly passed to the existing document
serializer so they cannot trigger its legacy unobserved-atomic whole-block
fallback. Empty text ranges retain an observation in the existing source
selection state; they contribute neither glyph highlighting, touch edges nor
code style. Excluded custom projections clear their observation instead of
creating a false empty endpoint that expands the copier's source span.

No extra document copy, geometry index or cache was introduced. Offscreen
endpoint resolution and persistent bidi affinity remain incomplete. These
changes have not been compiled or tested.


## Independent virtual selection boundaries

The existing cached block endpoints now track prefix/suffix preparation and
resolution separately. A bounded range across different known top-level blocks
projects each endpoint independently; an unchanged prepared endpoint retains its
AST selection while offscreen. Open-ended ranges share the same preparation.
Only represented non-boundary source owners are reset and filled together.

Changing an endpoint or its boundary role clears that block's stale source
selection once. Until native geometry is available, the range stays pending,
without borrowing another block's layout or applying the legacy whole-endpoint
copy fallback. Known complete interior blocks remain serializable even when
neither boundary has acquired a source observation. Paint retries pending
boundaries after registration and defers a redraw only on a preparation change.
Copy refreshes the same state synchronously. Logical snapshot comparison ignores
preparation metadata; swapping unchanged endpoints can reuse their preparation.
Local whole-view selection, table confinement and copy-document invalidation
invalidate preparation. No second text, AST or geometry cache was introduced.

Same-block ranges still require both native endpoint positions. First-time
unpainted endpoints and boundary-role changes while offscreen remain pending.
Whole-document replacement still needs window-selection lifecycle handling;
persistent bidi affinity and contextual soft wrapping remain outstanding.
These changes have not been compiled or tested.


## Retiring selection before content replacement

Rich-text content invalidation now retires the participant's old window
registration, hit-test runs, copy projections, source-boundary preparation and
painted selection edges. If that participant is an endpoint, a pending extension
anchor, part of a multi-click unit or inside the current window range, the window
gesture is cleared synchronously. Replacing an unrelated participant preserves
the existing window range. Retired geometry is not eligible for pointer input or
frame publication until fresh content registers again.

Window clearing can omit the replacing owner's view callback while still
clearing its base participant state. The caller already holds that view and
resets its document state directly; other clear callbacks retain synchronous
behavior. This avoids re-entering the borrowed owner and avoids a delayed clear
callback wiping a selection created after replacement.

External blocks, full text replacement, incompatible asynchronous parse commits
and native layout failure share this path. Full replacement retires immediately
before parsing, then incompatible commits retire gestures started on the visible
old baseline while parsing. Compatible appends and layout-only reflow keep their
existing selection-preserving paths. Existing frame vectors are cleared and
reused; no new cache, task or document representation was added.

This closes the previously recorded whole-document replacement lifecycle gap.
First-time unpainted endpoints, same-block absent geometry, offscreen boundary
role changes, persistent bidi affinity and contextual soft wrapping remain
outstanding. These changes have not been compiled or tested.

`TextViewState::clear_selection` now clears the owning participant’s window
range as well as its document and source-projection state. The shared ownership
check also serves content retirement, including pending extension anchors,
multi-click units and fully covered interior participants. Explicit clearing
retains unchanged hit geometry; unrelated window ranges remain selected.
The caller resets its own document synchronously and its clear callback is
omitted to avoid re-entering the borrowed view. Not compiled or tested yet.

Rich text endpoints now independently retain a weak source identity and UTF-8
byte offset when their own native fragment is available. A bounded range in
one top-level block and one text source can apply those offsets even after an
endpoint leaves the retained frame. Offsets are checked against live source
text before clearing old block selection; unrelated blocks are excluded.
The cache retains neither text nor geometry, transfers positions when logical
endpoints swap, and is retired with the existing content lifecycle. Initial
resolution rejects vertically absent fragments. Different source owners and
atomic endpoints retain the existing native projection path. Not compiled or
tested yet.

Source endpoint projection now also traverses different text owners and
inline objects within a top-level block, and rebuilds open prefix/suffix
ranges after an offscreen boundary changes role. The traversal follows the
existing paragraph text/object grouping and structured block groups, checks
source bounds before clearing, and prepares only selected canonical text
states. Single-chunk text is shared; existing matching text is retained.
Native inline object before/after rules provide weak atomic endpoints.
The common same-text-owner path remains direct. A partly resolved text/object
range stays pending rather than re-hitting its offscreen anchor against a
different visible fragment. Primitive custom-block endpoints retain their
existing policy. No source index, selection mirror or layout cache is added.
Not compiled or tested yet.

Compatible parsed updates now invalidate prepared source ranges and retire
the old adapter frame without cancelling the window gesture. Endpoints are
retained only if their source owner and byte boundary remain valid in the
committed tree. Replaced owners wait for fresh native geometry. This also
recomputes open ranges after appended content instead of retaining the old
document end. Parse failures keep the displayed baseline. Not compiled or
tested yet.

Primitive custom blocks now retain weak source identities and their existing
atomic coverage classification after native layout resolves an endpoint.
Open ranges, same-block bounded ranges and offscreen role changes project
back to the live tree. Two endpoints inside one plugin still do not select
the whole plugin; coverage edges are not text insertion points. A sole-source
root can resolve its before/after padding. Mixed roots reject padding without
the endpoint's own geometry. No retained block boxes or text copies are added.
Compatible updates validate these owners through the same source traversal.
Known-root open and bounded ranges now wait for their own source boundaries,
including two unresolved endpoints, instead of accepting a nearest visible
fragment as complete. Role changes let validated projection replace the old
range directly; unresolved roles clear it once while waiting.

Source projection clears out-of-range inline atoms to explicit exclusion,
including the direct same-text-owner path, so legacy source/Markdown copy
cannot infer adjacent unobserved images as selected. Explicit user clearing
continues to retire all observations. First-layout endpoints, ambiguous mixed
root padding, ordinary-window affinity, offscreen touch endpoints and native
wrap context remain open.
Not compiled or tested yet.

Resolved rich-text source endpoints now retain native hit affinity. A private
source-selection value carries real endpoint sides through reverse ranges,
tree projection, clipping and merging; artificial fragment cuts keep default
sides. Public input Selection and plain range APIs are unchanged. The common
hit helper resolves grapheme boundaries once and returns the native hit, so
the source path does not re-hit the same pointer just to obtain its side.

Native source caret boxes override painted fragment edges for touch geometry.
Painted edges reuse those resolved boxes instead of looking up the same
native caret twice.
An adjacent unselected fragment can supply its own upstream/downstream edge
without creating an empty selected range. Each edge resolves only in the
fragment which owns that logical side. Registration reuses the existing
current-frame geometry and retires it on frame reset. Zero-byte ranges produce
no handles. Fully covered flows clear old source metadata before merging their
full ranges. No text copies, source index or geometry cache is added. Ordinary
window-run affinity and genuinely offscreen touch geometry remain open.
Not compiled or tested yet.


## Contextual row cursor ownership

`gpui-base/src/text/inline_flow/prepared.rs`: contextual row preparation can reject a row after advancing its paragraph cursor. The ordinary `Rows::take` fallback now excludes contextual spans, so legacy rendering reshapes source text instead of re-entering the consumed cursor and tripping its source-order assertion. Successful contextual rows and strict native-layout failure handling retain their existing paths. No cursor clone, additional cache or persistent failure state is introduced. Linux desktop integration passes `cargo check -p super-paste-desktop --locked`; UI and other platforms remain unverified.

## Source-order row item traversal

`gpui-base/src/text/inline_flow.rs`: row assembly advances a local child index and source offset past consumed items instead of rescanning the entire prefix on every soft or hard row. Items overlapping a row remain on the ordinary fragment path; the text item containing a terminating newline is retained even for an empty row, and zero-byte box anchors continue through the existing box metrics. This removes repeated consumed-prefix traversal without another source index, text copy or retained cache. Rendering behavior remains subject to manual UI acceptance.
