# ReplayMod for Horizon

A Rust port of [ReplayMod](https://github.com/ReplayMod/replaymod) (GPL-3.0)
and its [ReplayStudio](https://github.com/ReplayMod/ReplayStudio) pathing
library (LGPL-3.0) to Horizon's native mod API. Goal: real ReplayMod
`.mcpr` replays and `.pttr` timelines load and play on Horizon.

## What is ported (1:1, tested)

- **Timeline model** (ReplayStudio `pathing`): Timeline/Path/Keyframe/
  PathSegment, all interpolators (linear, cubic spline, Catmull-Rom,
  polynomial spline), property groups, and the full undo change system.
- **Serialization**: `.pttr` JSON byte-compatible with upstream, including
  the legacy-timeline converter for old files.
- **SPTimeline**: the simple-pathing layer (TIME/POSITION paths, position
  properties) on top of the model.
- **Replay container**: `.mcpr` zip reading/writing (hand-rolled RFC 1951
  inflate, CRC32, entry change management) and the replay file metadata
  format (file format version 14).
- **Replay sender**: sync packet replay, the packet rewriter's
  `processPacket` dispatch (login/play transitions, camera entity, spectator
  gamemode, position limits, `replaymod:restrict` channel, resource-pack
  handling, bad-packet filtering) and the entity re-homing / vehicle settle
  logic that runs when a chunk unloads.
- **Keyframe path preview**: camera path sampling, spectator lines, the
  speed colour blend with its exponential easing, keyframe points.
- **Timeline players**: AbstractTimelinePlayer + RealtimeTimelinePlayer +
  ReplayTimer (second-frame clock anchor, overlay pause re-anchoring,
  tickDelta accumulation into ticksThisFrame, completion restore).
- **Extras**: entity-position tracker and player-overview logic.

23 smoke suites cover the ported logic.

## What had to change (Horizon seams)

- **Render pipeline**: ReplayMod's renderer (139 files) draws the replay
  offscreen to produce videos. Horizon has no offscreen render-to-video API,
  so the port emits the draw plan (camera path points, colours, lines) and
  the actual GL pass stays Horizon-side. Filed on horizon-rust/horizon,
  tracked until patched.
- **GUIs** (path editor, keyframe repository, timeline view): Horizon has no
  screen/widget surface for mods, so timeline editing happens through chat
  commands and config files for now.
- **Recording**: packet capture sits on Horizon's network stack; the capture
  hooks land with the mod's Horizon integration.
- **Resource packs** referenced by replays are resolved to their index
  entry; writing the pack files out is Horizon-side.
- `register()`/`unregister()` and the org-timer swap of the timeline player
  ride Horizon's tick loop.

## Licensing

Files derived from ReplayStudio carry LGPL-3.0 notices; files derived from
the ReplayMod client carry GPL-3.0 notices; both credit the ReplayMod
contributors. New files: Copyright (c) 2026 sparticle.
