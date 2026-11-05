//! Timeline players ported 1:1 from com.replaymod.pathing.player:
//! AbstractTimelinePlayer (155 ln), RealtimeTimelinePlayer (74 ln),
//! ReplayTimer (78 ln). Upstream (c) ReplayMod contributors,
//! GPL-3.0-or-later (https://github.com/ReplayMod/replaymod).
//! This file: Copyright (c) 2026 sparticle, GPL-3.0-or-later.
//!
//! Seams: ReplaySenderOps stands in for ReplayHandler.getReplaySender()
//! (the Horizon glue adapts the ported FullReplaySender to it); the overlay
//! query stands in for MinecraftClient.getOverlay(); the wall clock arrives
//! as now_ms. ReplayTimer's UpdatedCallback event is the game's per-frame
//! tick hook: the driver calls on_tick once per frame and then reads
//! ReplayTimer.ticks_this_frame. register()/unregister() and the org-timer
//! swap (MinecraftAccessor getTimer/setTimer) are Horizon seams; start()
//! installs the fresh ReplayTimer and the Restore action on completion is
//! where the glue restores the original timer.

use crate::pathing::{PropertyId, PropertyValue, Timeline};

/// Upstream Utils.DEFAULT_MS_PER_TICK (ReplayTimer tick length, 50 ms).
pub const DEFAULT_MS_PER_TICK: f64 = 50.0;

/// ReplayTimer port: a render tick counter that does not advance by itself.
/// tickDelta accumulates passed ticks each frame; ticksThisFrame is the
/// truncated whole-tick count the game consumes for that frame (the same
/// accumulation upstream onTick performs on the installed ReplayTimer).
#[derive(Default, Clone, Copy, Debug)]
pub struct ReplayTimer {
    pub tick_delta: f32,
    pub ticks_this_frame: i32,
}

impl ReplayTimer {
    pub fn new() -> Self { Self::default() }
    /// Upstream start(): timer.tickDelta = timer.ticksThisFrame = 0.
    pub fn reset(&mut self) { self.tick_delta = 0.0; self.ticks_this_frame = 0; }
}

/// The ReplaySender calls the player makes (upstream
/// replayHandler.getReplaySender()). The ported FullReplaySender adapts.
pub trait ReplaySenderOps {
    fn is_async_mode(&mut self) -> bool;
    fn set_sync_mode_and_wait(&mut self);
    fn set_async_mode(&mut self, async_mode: bool);
    fn set_replay_speed(&mut self, speed: f64);
    fn current_time_stamp(&mut self) -> i64;
}

/// AbstractTimelinePlayer port. The Java ListenableFuture<Void> becomes the
/// future_done flag; isActive() = started && !future_done.
pub struct AbstractTimelinePlayer {
    /// protected long startOffset in upstream.
    pub start_offset: i64,
    was_async_mode: bool,
    last_time: i64,
    last_timestamp: i64,
    started: bool,
    future_done: bool,
}

#[derive(Debug, PartialEq)]
pub enum TickAction {
    /// Normal frame: the game consumes ReplayTimer.ticks_this_frame.
    Continue,
    /// Future done: restore the org timer, setReplaySpeed(0), restore async
    /// mode and unregister (upstream onTick's done branch).
    Restore,
}

impl AbstractTimelinePlayer {
    pub fn new() -> Self {
        Self { start_offset: 0, was_async_mode: false, last_time: 0,
               last_timestamp: 0, started: false, future_done: false }
    }

    /// Upstream start(Timeline, long from) + start(Timeline): lastTimestamp =
    /// the max keyframe time across all paths (0 when there are none),
    /// sender mode forced to sync, fresh timer, future created.
    pub fn start(&mut self, timeline: &Timeline, ops: &mut dyn ReplaySenderOps,
                 timer: &mut ReplayTimer, from: i64) {
        self.start_offset = from;
        self.last_timestamp = timeline.paths.iter()
            .flat_map(|p| p.keyframes.keys().copied())
            .max()
            .unwrap_or(0);
        self.was_async_mode = ops.is_async_mode();
        ops.set_sync_mode_and_wait();
        // register() and the org-timer swap are Horizon seams; the port
        // installs a fresh ReplayTimer, matching upstream's
        // timer.tickDelta = timer.ticksThisFrame = 0.
        timer.reset();
        self.started = true;
        self.future_done = false;
        self.last_time = 0;
    }

    pub fn is_active(&self) -> bool { self.started && !self.future_done }

    /// Upstream onTick body minus the getTimePassed() call, which the
    /// concrete player supplies. time_passed is getTimePassed()'s value.
    pub fn on_tick(&mut self, time_passed: i64, timeline: &mut Timeline,
                   ops: &mut dyn ReplaySenderOps, timer: &mut ReplayTimer,
                   applier: &mut dyn FnMut(PropertyId, &PropertyValue)) -> TickAction {
        if self.future_done {
            ops.set_replay_speed(0.0);
            if self.was_async_mode {
                ops.set_async_mode(true);
            }
            // unregister() + restore org timer: Horizon seam.
            return TickAction::Restore;
        }
        let mut time = time_passed;
        if time > self.last_timestamp {
            time = self.last_timestamp;
        }

        // Apply to timeline
        let _ = timeline.apply_to_game(time, applier);
        // Apply a second time in case some of the packets have moved the
        // camera from where it was (upstream comment).
        let _ = timeline.apply_to_game(time, applier);

        // Update minecraft timer
        let replay_time = ops.current_time_stamp();
        if self.last_time == 0 {
            // First frame, no change yet
            self.last_time = replay_time;
        }
        let time_in_ticks = replay_time as f32 / 50f32;
        let previous_time_in_ticks = self.last_time as f32 / 50f32;
        let passed_ticks = time_in_ticks - previous_time_in_ticks;
        timer.tick_delta += passed_ticks;
        timer.ticks_this_frame = timer.tick_delta as i32;
        timer.tick_delta -= timer.ticks_this_frame as f32;

        self.last_time = replay_time;

        if time >= self.last_timestamp {
            self.future_done = true;
        }
        TickAction::Continue
    }
}

/// RealtimeTimelinePlayer port: plays the timeline against the system clock.
pub struct RealtimeTimelinePlayer {
    base: AbstractTimelinePlayer,
    /// Whether the next frame is the first frame. We only start measuring
    /// time from the second frame as the first might have to jump in time
    /// which might take time (upstream comment).
    first_frame: bool,
    second_frame: bool,
    /// System time in milliseconds at the start.
    start_time: i64,
    loading_resources: bool,
    time_before_resource_loading: i64,
}

impl RealtimeTimelinePlayer {
    pub fn new() -> Self {
        Self { base: AbstractTimelinePlayer::new(), first_frame: false,
               second_frame: false, start_time: 0, loading_resources: false,
               time_before_resource_loading: 0 }
    }

    /// Upstream start(Timeline): reset the frame tracking, then super.start.
    pub fn start(&mut self, timeline: &Timeline, ops: &mut dyn ReplaySenderOps,
                 timer: &mut ReplayTimer, from: i64) {
        self.first_frame = true;
        self.loading_resources = false;
        self.base.start(timeline, ops, timer, from);
    }

    pub fn is_active(&self) -> bool { self.base.is_active() }
    pub fn start_offset(&self) -> i64 { self.base.start_offset }

    /// Upstream onTick. overlay_shown stands in for
    /// MinecraftClient.getInstance().getOverlay() != null (the MC>=11400
    /// branch); when the overlay is up, playback time freezes and re-anchors
    /// once it clears.
    pub fn on_tick(&mut self, timeline: &mut Timeline, ops: &mut dyn ReplaySenderOps,
                   timer: &mut ReplayTimer,
                   applier: &mut dyn FnMut(PropertyId, &PropertyValue),
                   now_ms: i64, overlay_shown: bool) -> TickAction {
        if self.second_frame {
            self.second_frame = false;
            self.start_time = now_ms - self.base.start_offset;
        }

        if overlay_shown {
            if !self.loading_resources {
                self.time_before_resource_loading = self.get_time_passed(now_ms);
                self.loading_resources = true;
            }
            return self.base.on_tick(self.get_time_passed(now_ms), timeline,
                                     ops, timer, applier);
        } else if self.loading_resources && !self.first_frame {
            self.start_time = now_ms - self.time_before_resource_loading;
            self.loading_resources = false;
        }

        let action = self.base.on_tick(self.get_time_passed(now_ms), timeline,
                                       ops, timer, applier);

        if self.first_frame {
            self.first_frame = false;
            self.second_frame = true;
        }
        action
    }

    /// Upstream getTimePassed().
    fn get_time_passed(&self, now_ms: i64) -> i64 {
        if self.first_frame { return self.base.start_offset; }
        if self.loading_resources { return self.time_before_resource_loading; }
        now_ms - self.start_time
    }
}
