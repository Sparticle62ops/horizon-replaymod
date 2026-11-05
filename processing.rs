//! Sync-mode packet processing: 1:1 Rust port of ReplayMod's
//! FullReplaySender.processPacket (LGPL-3.0-or-later), active MC>=11400
//! branch. This file: Copyright (c) 2026 sparticle, LGPL-3.0-or-later.
//!
//! The MC packet model below is the Horizon-integration seam: a decoded
//! packet carries the upstream Java class name as `kind` plus the fields
//! processPacket reads or rewrites. Raw MC decode/encode is Horizon-side.
use std::collections::BTreeMap;

/// Upstream Restrictions.PLUGIN_CHANNEL (1.14+ Identifier form).
pub const PLUGIN_CHANNEL: &str = "replaymod:restrict";
/// Upstream GameJoin rewrite: "Camera entity id should be negative which is
/// an invalid id and can't be used by servers".
pub const CAMERA_ENTITY_ID: i32 = -1789435;
/// Upstream TP_DISTANCE_LIMIT.
pub const TP_DISTANCE_LIMIT: i32 = 128;
/// net.minecraft.world.GameMode.SPECTATOR wire id.
pub const GAME_MODE_SPECTATOR: i32 = 3;
/// PlayerPositionLookS2CPacket.Flag X/Y/Z bit values (relative coords).
pub const FLAG_X: u32 = 0x01;
pub const FLAG_Y: u32 = 0x02;
pub const FLAG_Z: u32 = 0x04;

/// Upstream BAD_PACKETS, MC>=11400 && <12005 active branch: LoginHelloS2CPacket,
/// PlayerActionResponseS2CPacket, OpenWrittenBookS2CPacket, OpenScreenS2CPacket,
/// SynchronizeRecipesS2CPacket, AdvancementUpdateS2CPacket,
/// SelectAdvancementTabS2CPacket, SetCameraEntityS2CPacket, TitleS2CPacket,
/// HealthUpdateS2CPacket, OpenHorseScreenS2CPacket, CloseScreenS2CPacket,
/// ScreenHandlerSlotUpdateS2CPacket, ScreenHandlerPropertyUpdateS2CPacket,
/// SignEditorOpenS2CPacket, StatisticsS2CPacket, ExperienceBarUpdateS2CPacket,
/// PlayerAbilitiesS2CPacket. (ServerTransferS2CPacket is the MC>=12005
/// preprocessor branch, not in this target.)
pub fn is_bad_packet(kind: &str) -> bool {
    matches!(kind,
        "LoginHelloS2CPacket" | "PlayerActionResponseS2CPacket"
        | "OpenWrittenBookS2CPacket" | "OpenScreenS2CPacket"
        | "SynchronizeRecipesS2CPacket" | "AdvancementUpdateS2CPacket"
        | "SelectAdvancementTabS2CPacket" | "SetCameraEntityS2CPacket"
        | "TitleS2CPacket" | "HealthUpdateS2CPacket" | "OpenHorseScreenS2CPacket"
        | "CloseScreenS2CPacket" | "ScreenHandlerSlotUpdateS2CPacket"
        | "ScreenHandlerPropertyUpdateS2CPacket" | "SignEditorOpenS2CPacket"
        | "StatisticsS2CPacket" | "ExperienceBarUpdateS2CPacket"
        | "PlayerAbilitiesS2CPacket")
}

/// Upstream packet-type registry state (getPacketTypeRegistry(State.LOGIN)
/// -> withLoginSuccess() transitions to PLAY).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryState { Login, Play }

/// Sender state processPacket mutates (upstream fields with the same names).
#[derive(Clone, Debug)]
pub struct ReplaySenderState {
    pub registry: RegistryState,
    pub actual_id: i32,
    pub allow_movement: bool,
    pub has_world_loaded: bool,
}

impl Default for ReplaySenderState {
    fn default() -> Self {
        Self { registry: RegistryState::Login, actual_id: -1, allow_movement: false, has_world_loaded: false }
    }
}

/// The decoded packet: `kind` is the upstream Java class name; payload holds
/// exactly the fields processPacket touches.
#[derive(Clone, Debug, PartialEq)]
pub struct Packet {
    pub kind: String,
    pub payload: Payload,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
    None,
    CustomPayload { channel: String, data: Vec<u8> },
    Disconnect { reason: String },
    ResourcePackSend { url: String, pack_id: i32 },
    /// 1.14-1.16 branch: entity id + (1.16+) both game-mode fields.
    GameJoin { entity_id: i32, mode: i32, mode_prev: i32 },
    PlayerRespawn { mode: i32, mode_prev: i32 },
    PlayerPositionLook { flags: u32 },
    GameStateChange { reason: i32, value: f32 },
    GameMessage,
}

/// Upstream returns null to drop the packet; restrictions parse failure
/// terminates the replay outright.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Keep(Packet),
    Drop,
    Terminate,
}

/// `restrictions` is the Restrictions.handle seam: Some(unknown tag) on a
/// parse failure, None when the options parsed. `rp_index` is the replay's
/// resourcepack index (phase-7 accessors).
pub fn process_packet(
    st: &mut ReplaySenderState,
    p: Packet,
    rp_index: Option<&BTreeMap<i32, String>>,
    show_chat: bool,
    suppress_camera: bool,
    restrictions: &dyn Fn(&[u8]) -> Option<String>,
) -> Outcome {
    // LoginSuccessS2CPacket: registry = registry.withLoginSuccess()
    if p.kind == "LoginSuccessS2CPacket" {
        st.registry = RegistryState::Play;
        return Outcome::Keep(p);
    }

    // CustomPayloadS2CPacket on the restrictions channel: parsed -> drop;
    // unknown -> terminate the replay (upstream tears it down in runLater).
    if let Payload::CustomPayload { channel, data } = &p.payload {
        if channel == PLUGIN_CHANNEL {
            return match restrictions(data) {
                None => Outcome::Drop,
                Some(_unknown) => Outcome::Terminate,
            };
        }
    }

    // DisconnectS2CPacket: drop the forced-update disconnect, keep others.
    if let Payload::Disconnect { reason } = &p.payload {
        if reason == "Please update to view this replay." {
            return Outcome::Drop;
        }
    }

    // BAD_PACKETS filter.
    if is_bad_packet(&p.kind) {
        return Outcome::Drop;
    }

    // Modded screens opened for the recording player are not shown.
    if let Payload::CustomPayload { channel, .. } = &p.payload {
        if channel.starts_with("fabric-screen-handler-api-v") {
            return Outcome::Drop;
        }
    }

    // ResourcePackSendS2CPacket: replay:// packs resolve through the index
    // and are materialized Horizon-side; the packet itself is dropped.
    if let Payload::ResourcePackSend { url, pack_id } = &p.payload {
        if url.starts_with("replay://") {
            let id: i32 = url["replay://".len()..].parse().unwrap_or(0);
            if let Some(index) = rp_index {
                if index.contains_key(&id) {
                    // Upstream extracts resourcePack(hash).zip into the temp
                    // folder and hands it to the server resource pack provider:
                    // Horizon integration seam.
                    let _ = pack_id;
                }
            }
            return Outcome::Drop;
        }
    }

    // GameJoinS2CPacket: camera entity id rewrite + SPECTATOR mode.
    if let Payload::GameJoin { entity_id, mode, mode_prev } = p.payload {
        st.allow_movement = true; // upstream schedules this
        st.actual_id = entity_id;
        let _ = (mode, mode_prev); // upstream keeps every other field as-is
        return Outcome::Keep(Packet {
            kind: p.kind,
            payload: Payload::GameJoin {
                entity_id: CAMERA_ENTITY_ID,
                mode: GAME_MODE_SPECTATOR,
                mode_prev: GAME_MODE_SPECTATOR,
            },
        });
    }

    // PlayerRespawnS2CPacket: SPECTATOR mode on respawn too.
    if let Payload::PlayerRespawn { mode: _, mode_prev: _ } = p.payload {
        st.allow_movement = true; // upstream schedules this
        return Outcome::Keep(Packet {
            kind: p.kind,
            payload: Payload::PlayerRespawn {
                mode: GAME_MODE_SPECTATOR,
                mode_prev: GAME_MODE_SPECTATOR,
            },
        });
    }

    // PlayerPositionLookS2CPacket: the camera moves are applied Horizon-side;
    // the packet is dropped after state bookkeeping.
    if let Payload::PlayerPositionLook { flags } = p.payload {
        st.has_world_loaded = true;
        if suppress_camera {
            return Outcome::Drop;
        }
        if flags & (FLAG_X | FLAG_Y | FLAG_Z) != 0 {
            // At least one of the coordinates is relative, so we don't care.
            return Outcome::Drop;
        }
        st.allow_movement = false; // upstream: allowMovement = false unless TP
        return Outcome::Drop;
    }

    // GameStateChangeS2CPacket: only rain controls pass through
    // (1 RAIN_STOPPED, 2 RAIN_STARTED, 7 RAIN_GRADIENT_CHANGED,
    // 8 THUNDER_GRADIENT_CHANGED).
    if let Payload::GameStateChange { reason, value: _ } = p.payload {
        if !matches!(reason, 1 | 2 | 7 | 8) {
            return Outcome::Drop;
        }
        return Outcome::Keep(p);
    }

    // GameMessageS2CPacket: dropped when chat is hidden in the settings.
    if p.payload == Payload::GameMessage && !show_chat {
        return Outcome::Drop;
    }

    Outcome::Keep(p)
}
