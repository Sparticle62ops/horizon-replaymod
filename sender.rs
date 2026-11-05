//! Replay sender: 1:1 Rust port of ReplayMod's ReplaySender interface and
//! FullReplaySender's asynchronous packet-pacing loop (the replay playback
//! engine). Upstream: com.replaymod.replay.ReplaySender (35 ln) and
//! com.replaymod.replay.FullReplaySender's async half (constructor, mode
//! switching, getReplaySpeed/setReplaySpeed, asyncSender runnable, hurry/
//! jump logic; LGPL-3.0-or-later). This file: Copyright (c) 2026 sparticle,
//! LGPL-3.0-or-later.
//!
//! Transport-bound pieces (Netty channelRead, Minecraft timer tick-length
//! manipulation, processPacket's 400 lines of per-packet MC rewriting, the
//! sync-mode sendPacketsTill stream walk) are Horizon-integration work and
//! stay behind the traits/hook notes below; the pacing state machine is
//! ported 1:1 and driven by tick(now_ms) instead of threads so it is
//! testable headless.
use crate::pathing::*; // keep module set uniform

/// Upstream PacketData: a recorded packet with its replay timestamp.
/// is_bundle marks PacketType.Bundle toggles (bundle batching).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PacketKind { Particle, EntitySpawnFireworkRocket, Other }

#[derive(Clone, Debug)]
pub struct PacketData { pub timestamp: i32, pub is_bundle: bool, pub kind: PacketKind, pub bytes: Vec<u8> }

/// Upstream ReplayFile.getPacketData + ReplayInputStream, as a hook.
pub trait PacketSource {
    /// Upstream new PacketData(replayIn): next recorded packet, or None at
    /// EOF (upstream EOFException), Err on a read failure (IOException).
    fn next_packet(&mut self) -> Result<PacketData, String>;
    /// Upstream reopens replayIn on restart
    /// (replayFile.getPacketData(...)); reopen the underlying stream.
    fn reopen(&mut self);
}

/// Upstream channel.pipeline().fireChannelRead(...): deliver bytes into the
/// client's channel. Horizon: its connection's packet ingress.
pub trait PacketSink { fn deliver(&mut self, bytes: &[u8]); }

/// Upstream ReplayHandler hooks the async loop calls.
pub trait ReplayHandler {
    /// moveCameraToTargetPosition after a jump completes.
    fn move_camera_to_target_position(&mut self);
    /// restartedReplay, run on restart (upstream wraps in runSync).
    fn restarted_replay(&mut self);
}

/// One iteration's outcome, replacing Thread.sleep/continue control flow so
/// the driver (a thread loop in production, a test in CI) decides pacing.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LoopAction {
    /// Sleep the given ms and retry the iteration (upstream Thread.sleep).
    Sleep(u64),
    /// A packet was delivered (or the loop moved forward) this iteration.
    Progress,
    /// The replay loop terminated (upstream break REPLAY_LOOP).
    Terminated,
}

/// Upstream ReplaySender + FullReplaySender's async state.
pub struct FullReplaySender {
    pub async_mode: bool,
    /// Upstream lastTimeStamp: timestamp of the last delivered packet.
    pub last_time_stamp: i32,
    pub current_time_stamp: i32,
    pub replay_speed: f64,
    pub has_world_loaded: bool,
    pub in_bundle: bool,
    pub start_from_beginning: bool,
    pub terminate: bool,
    /// realTimeStart: ms anchor for timing (upstream keeps this so pauses and
    /// speed changes don't cause gradual drift).
    pub real_time_start: u64,
    pub real_time_start_speed: f64,
    /// desiredTimeStamp: -1 means normal timing; otherwise rushing.
    pub desired_time_stamp: i64,
    pub replay_length: i32,
    /// Upstream nextPacket: read ahead but not yet delivered.
    next_packet: Option<PacketData>,
    /// Whether a replayIn stream is open (upstream lazy-opens it).
    stream_open: bool,
    /// next_packet source has been consumed to EOF at least once.
    at_eof: bool,
}

impl FullReplaySender {
    pub fn new(replay_length: i32) -> Self {
        Self {
            async_mode: false,
            last_time_stamp: 0,
            current_time_stamp: 0,
            replay_speed: 1.0,
            has_world_loaded: false,
            in_bundle: false,
            start_from_beginning: false,
            terminate: false,
            real_time_start: 0,
            real_time_start_speed: 1.0,
            desired_time_stamp: -1,
            replay_length,
            next_packet: None,
            stream_open: false,
            at_eof: false,
        }
    }

    /// Upstream paused(): tickLength == POSITIVE_INFINITY, i.e. speed 0.
    pub fn paused(&self) -> bool { self.replay_speed == 0.0 }

    /// Upstream getReplaySpeed.
    pub fn get_replay_speed(&self) -> f64 {
        if !self.paused() { self.replay_speed } else { 0.0 }
    }

    /// Upstream setReplaySpeed: re-anchor realTimeStart so the wall-clock
    /// mapping stays drift-free; d == 0 just pauses. The Minecraft timer
    /// tick-length change is a Horizon hook, not ported.
    pub fn set_replay_speed(&mut self, d: f64, now_ms: u64) {
        if d != 0.0 {
            self.replay_speed = d;
            self.real_time_start_speed = d;
            self.real_time_start = now_ms.saturating_sub((self.last_time_stamp as f64 / d) as u64);
        }
        self.replay_speed = if d != 0.0 { d } else { 0.0 };
    }

    /// Upstream isHurrying.
    pub fn is_hurrying(&self) -> bool { self.desired_time_stamp != -1 }

    /// Upstream stopHurrying.
    pub fn stop_hurrying(&mut self) { self.desired_time_stamp = -1; }

    /// Upstream getDesiredTimestamp.
    pub fn get_desired_timestamp(&self) -> i64 { self.desired_time_stamp }

    /// Upstream jumpToTime (async mode only): rushing until a packet past the
    /// target is reached; backwards jumps restart from the beginning.
    pub fn jump_to_time(&mut self, millis: i32) -> Result<(), String> {
        if !self.async_mode {
            return Err("Can only jump in async mode. Use sendPacketsTill(int) instead.".into());
        }
        if millis < self.last_time_stamp && !self.is_hurrying() {
            self.start_from_beginning = true;
        }
        self.desired_time_stamp = millis as i64;
        Ok(())
    }

    /// Upstream terminateReplay's loop-exit flag.
    pub fn terminate_replay(&mut self) { self.terminate = true; }

    /// Upstream asyncSender's REPLAY_LOOP body, one iteration. The driver
    /// calls this repeatedly; Sleep(n) means wait n ms then retry. The
    /// restart block (upstream's outer while) is restart_state_reset().
    pub fn tick(
        &mut self,
        now_ms: u64,
        source: &mut dyn PacketSource,
        sink: &mut dyn PacketSink,
        handler: &mut dyn ReplayHandler,
    ) -> Result<LoopAction, String> {
        if !self.async_mode {
            return Err("async loop only runs in async mode".into());
        }
        // Upstream: lazy-open the packet stream.
        if !self.stream_open { self.stream_open = true; }

        // Upstream: while paused and world loaded and not bundling, sleep,
        // unless terminating, restarting or jumping.
        while self.paused() && self.has_world_loaded && !self.in_bundle {
            if self.terminate || self.start_from_beginning || self.desired_time_stamp != -1 {
                break;
            }
            return Ok(LoopAction::Sleep(10));
        }

        if self.terminate && !self.in_bundle {
            return Ok(LoopAction::Terminated);
        }

        if self.start_from_beginning {
            // Upstream: break out of the packet loop so the outer loop
            // restarts the replay.
            return Ok(LoopAction::Progress);
        }

        // Read the next packet if we don't already have one.
        if self.next_packet.is_none() {
            if self.at_eof {
                // Upstream EOFException path: pause and wait until told to
                // continue (jump/restart/terminate).
                self.set_replay_speed(0.0, now_ms);
                if self.paused() && self.has_world_loaded && self.desired_time_stamp == -1 && !self.terminate {
                    return Ok(LoopAction::Sleep(10));
                }
                if self.terminate { return Ok(LoopAction::Terminated); }
                return Ok(LoopAction::Progress); // break -> restart block
            }
            match source.next_packet() {
                Ok(p) => { self.next_packet = Some(p); self.at_eof = false; }
                Err(e) if e == "EOF" => {
                    // Upstream: pause, then wait until the user continues.
                    self.set_replay_speed(0.0, now_ms);
                    self.at_eof = true;
                    return Ok(LoopAction::Sleep(10));
                }
                Err(e) => return Err(e), // upstream: printStackTrace, retry
            }
        }

        let packet = self.next_packet.clone().expect("next_packet just set");
        let next_time_stamp = packet.timestamp;

        // Upstream: if not hurrying and world loaded (no dirt-screens), wait
        // for proper packet timing.
        if !self.is_hurrying() && self.has_world_loaded && !self.in_bundle {
            let expected_time = self.real_time_start + (next_time_stamp as f64 / self.replay_speed) as u64;
            if self.replay_speed != 0.0 && expected_time > now_ms {
                return Ok(LoopAction::Sleep(expected_time - now_ms));
            }
        }

        // Process packet.
        if packet.is_bundle { self.in_bundle = !self.in_bundle; }
        sink.deliver(&packet.bytes);
        self.next_packet = None;
        self.last_time_stamp = next_time_stamp;

        // Upstream (>=1.20.2): stall while autoRead is disabled; Horizon hook,
        // not ported.

        // Upstream: when a jump finishes, snap the camera and pause.
        if self.is_hurrying() && self.last_time_stamp as i64 > self.desired_time_stamp && !self.start_from_beginning {
            self.desired_time_stamp = -1;
            handler.move_camera_to_target_position();
            self.set_replay_speed(0.0, now_ms);
        }
        Ok(LoopAction::Progress)
    }

    /// Upstream's restart block (outer loop bottom): reset per-run state and
    /// reopen the stream. Call after tick() returns Progress with
    /// start_from_beginning set (or after EOF-continue).
    pub fn restart_state_reset(&mut self, now_ms: u64, source: &mut dyn PacketSource, handler: &mut dyn ReplayHandler) {
        self.has_world_loaded = false;
        self.in_bundle = false;
        self.last_time_stamp = 0;
        self.start_from_beginning = false;
        self.next_packet = None;
        self.at_eof = false;
        self.stream_open = false;
        self.real_time_start = now_ms;
        source.reopen();
        handler.restarted_replay();
    }

    /// Upstream currentTimeStamp (async): last delivered packet's timestamp.
    pub fn current_time_stamp(&self) -> i32 { self.last_time_stamp }

    /// Upstream processPacketAsync: while hurrying over a long span, drop
    /// particle packets and firework-rocket spawns (except for short
    /// durations) so the rush doesn't lag the client.
    pub fn process_packet_async(&self, p: &PacketData) -> Option<PacketData> {
        if self.desired_time_stamp - self.last_time_stamp as i64 > 1000 {
            if p.kind == PacketKind::Particle { return None; }
            if p.kind == PacketKind::EntitySpawnFireworkRocket { return None; }
        }
        Some(p.clone())
    }

    /// Upstream sendPacketsTill (sync mode): send all packets up to and
    /// including the target timestamp, no pacing. The upstream single-thread
    /// executor + MC task-queue drain is a Horizon hook, not ported; this
    /// runs the send inline.
    pub fn send_packets_till(
        &mut self,
        timestamp: i32,
        now_ms: u64,
        source: &mut dyn PacketSource,
        sink: &mut dyn PacketSink,
        handler: &mut dyn ReplayHandler,
    ) -> Result<(), String> {
        if self.async_mode {
            return Err("This method cannot be used in async mode. Use jumpToTime(int) instead.".into());
        }
        self.do_send_packets_till(timestamp, now_ms, source, sink, handler)
    }

    fn do_send_packets_till(
        &mut self,
        timestamp: i32,
        now_ms: u64,
        source: &mut dyn PacketSource,
        sink: &mut dyn PacketSink,
        handler: &mut dyn ReplayHandler,
    ) -> Result<(), String> {
        if timestamp == self.last_time_stamp { return Ok(()); } // already there
        if timestamp < self.last_time_stamp {
            // Upstream: restart the replay to go backwards in time.
            self.has_world_loaded = false;
            self.in_bundle = false;
            self.last_time_stamp = 0;
            self.stream_open = false;
            self.start_from_beginning = false;
            self.next_packet = None;
            self.at_eof = false;
            source.reopen();
            handler.restarted_replay();
        }
        self.stream_open = true;

        loop { // Upstream: send packets
            // Leftover packet from before is used first.
            let pd = match self.next_packet.take() {
                Some(p) => p,
                None => match source.next_packet() {
                    Ok(p) => p,
                    Err(e) if e == "EOF" => {
                        // Upstream: "we hit the end before finishing our job...
                        // well, let's just pretend we're done"
                        self.stream_open = false;
                        break;
                    }
                    Err(e) => return Err(e),
                },
            };
            let next_time_stamp = pd.timestamp;
            if next_time_stamp > timestamp && !self.in_bundle {
                // Done sending all packets; keep the packet for later.
                self.next_packet = Some(pd);
                break;
            }
            if pd.is_bundle { self.in_bundle = !self.in_bundle; }
            sink.deliver(&pd.bytes);
            // Upstream >=1.20.2 autoRead stall: Horizon hook, not ported.
        }

        // Upstream: re-anchor realTimeStart in case we switch to async soon.
        self.real_time_start = now_ms.saturating_sub((timestamp as f64 / self.replay_speed) as u64);
        self.last_time_stamp = timestamp;
        Ok(())
    }
}
