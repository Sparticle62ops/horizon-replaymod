# replaymod port plan (native Horizon mod)

Upstream sources read 2026-11-04:
- ReplayMod client: https://github.com/ReplayMod/replaymod (GPL-3.0, 467 Java files)
- ReplayStudio library: https://github.com/ReplayMod/ReplayStudio (LGPL-3.0, 143 Java files)

## Package map (replaymod)
| pkg | files | lines | verdict |
|---|---|---|---|
| core | 66 | 5778 | mostly Minecraft/GUI glue; port helpers selectively |
| simplepathing | 10 | 3619 | SPTimeline + GuiPathing; SPTimeline portable 1:1 |
| pathing | 7 | 963 | properties + timeline players, portable 1:1 |
| replay | 63 | 6696 | ReplaySender/QuickReplaySender portable; GUI degrades |
| recording | 33 | 2776 | packet capture; netty-bound, port capture logic |
| render | 139 | 9637 | offscreen render pipeline; Horizon has no render-to-video API -> degrade + issue |
| editor | 4 | 608 | GUI-bound |
| extras | 18 | 1718 | youtube/screenshots -> degrade; playeroverview logic portable |
| compat | 13 | 230 | skip (Forge/OptiFine/shaders) |

## Phase 1: ReplayStudio pathing package (36 files, 3721 lines, pure logic, no Minecraft deps)
- path/path: Timeline(121), Path(134), Keyframe(73), PathSegment(63)
- path/impl: TimelineImpl(119), PathImpl(204), KeyframeImpl(61), PathSegmentImpl(53)
- interpolation: Interpolator(119), AbstractInterpolator(102), LinearInterpolator(125), CatmullRomSplineInterpolator(221), CubicSplineInterpolator(97), PolynomialSplineInterpolator(280), InterpolationParameters(46)
- property: Property(100), PropertyParts(115), PropertyGroup(68), abstracts(150)
- change (undo): Change + AddKeyframe/RemoveKeyframe/SetInterpolator/UpdateKeyframeProperties/CombinedChange/AddPath/RemovePath/Add+RemoveInterpolatorProperty (~640)
- serialize: TimelineSerialization(247) - JSON format MUST stay byte-compatible with upstream .pttr files; LegacyTimelineConverter(175); LegacyKeyframeSetAdapter(146)

## Phase 2: replaymod logic layer
- SPTimeline.java (695) 1:1 on top of the Phase 1 model; SPPath TIME/POSITION paths
- pathing/properties (CameraProperties 118, TimestampProperty 50, SpectatorProperty 64)
- pathing/player: AbstractTimelinePlayer(155), RealtimeTimelinePlayer(74), ReplayTimer(78)
- replay/file + ReplaySender: needs Horizon protocol packet replay surface

## Phase 3: recording + rendering
- recording: packet capture on Horizon's net stack
- render: NO Horizon offscreen render-to-video API -> document degradation in README, file issue/PR on horizon-rust/horizon per standing policy, track until patched
- GUIs (path editor, keyframe repository, timeline view): follow Horizon UI conventions, degrade where the API lacks widgets

## Licensing
- ReplayStudio-derived files: LGPL-3.0 notice + credit ReplayMod contributors
- replaymod-derived files: GPL-3.0 notice + credit
- protocol/file-format compat is the goal (real ReplayMod .mcpr/.pttr files must load)
