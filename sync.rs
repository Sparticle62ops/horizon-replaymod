//! Sync-mode entity handling on chunk unload: 1:1 FullReplaySender
//! processPacketSync (the MC>=11404 branch) + forcePositionForVehicleAndSelf.
//! Upstream drives the live Minecraft world; the algorithm is ported over a
//! minimal entity/chunk model so it runs and tests headless. Movement and
//! vehicle-follow behavior live behind tick() / tick_riding().
//! This file: Copyright (c) 2026 sparticle, LGPL-3.0-or-later.
use std::collections::BTreeMap;

const SETTLE_EPS2: f64 = 0.0001; // prevPos.squaredDistanceTo(...) > 0.0001
const SETTLE_MAX_TICKS: u32 = 100; // ticks++ < 100

#[derive(Clone)]
pub struct SyncEntity {
    pub id: i32,
    pub x: f64, pub y: f64, pub z: f64,
    pub chunk_x: i32, pub chunk_y: i32, pub chunk_z: i32,
    pub update_needed: bool,
    pub vehicle: Option<i32>,
    // Stand-in for the vanilla tick: keeps moving by vel for settle_ticks
    // ticks, then rests (so the settle loop has something to converge on).
    pub vel: (f64, f64, f64),
    pub settle_ticks: u32,
}

#[derive(Default)]
pub struct SyncWorld {
    pub entities: BTreeMap<i32, SyncEntity>,
    // (chunkX, chunkZ) -> entity ids registered in that chunk.
    pub chunks: BTreeMap<(i32, i32), Vec<i32>>,
}

fn floor16(v: f64) -> i32 { (v / 16.0).floor() as i32 }

impl SyncWorld {
    pub fn add_entity(&mut self, e: SyncEntity, registered_chunk: Option<(i32, i32)>) {
        let id = e.id;
        if let Some((cx, cz)) = registered_chunk {
            self.chunks.entry((cx, cz)).or_default().push(id);
        }
        self.entities.insert(id, e);
    }

    /// vanilla Entity.tick(): the entity's own movement for one tick.
    fn tick(&mut self, id: i32) {
        let e = self.entities.get_mut(&id).unwrap();
        if e.settle_ticks > 0 {
            e.x += e.vel.0; e.y += e.vel.1; e.z += e.vel.2;
            e.settle_ticks -= 1;
        }
    }

    /// vanilla Entity.tickRiding(): carried by the vehicle's movement.
    fn tick_riding(&mut self, id: i32) {
        let vid = self.entities[&id].vehicle;
        if let Some(v) = vid {
            let (vx, vy, vz) = { let ve = &self.entities[&v]; (ve.x, ve.y, ve.z) };
            let e = self.entities.get_mut(&id).unwrap();
            e.x = vx; e.y = vy; e.z = vz;
        }
    }

    /// 1:1 forcePositionForVehicleAndSelf: settle the whole vehicle chain,
    /// deepest vehicle first, ticking each until its position stabilizes.
    pub fn force_position_for_vehicle_and_self(&mut self, id: i32) {
        let vehicle = self.entities[&id].vehicle;
        if let Some(v) = vehicle {
            self.force_position_for_vehicle_and_self(v);
        }
        let mut ticks: u32 = 0;
        loop {
            let (px, py, pz) = { let e = &self.entities[&id]; (e.x, e.y, e.z) };
            if vehicle.is_some() { self.tick_riding(id); } else { self.tick(id); }
            let e = &self.entities[&id];
            let d2 = (e.x - px) * (e.x - px) + (e.y - py) * (e.y - py) + (e.z - pz) * (e.z - pz);
            // Java do-while: ticks++ runs on EVERY condition evaluation.
            let cont = d2 > SETTLE_EPS2 && ticks < SETTLE_MAX_TICKS;
            ticks += 1;
            if !cont { break; }
        }
    }

    /// 1:1 processPacketSync for UnloadChunkS2CPacket (MC>=10904 / >=11400
    /// branch): before a chunk unloads, force-update every entity registered
    /// in it (skipping server-position interpolation), then re-home any that
    /// ticked themselves into a different chunk.
    pub fn process_packet_sync(&mut self, x: i32, z: i32) {
        let Some(ids) = self.chunks.get(&(x, z)).cloned() else { return; };
        for id in ids {
            if !self.entities.contains_key(&id) { continue; }
            // Skip interpolation of position updates coming from server.
            self.force_position_for_vehicle_and_self(id);
            // MC>=11404: leaving the chunk is now checked here.
            let e = &self.entities[&id];
            let cx = floor16(e.x); let cy = floor16(e.y); let cz = floor16(e.z);
            if e.chunk_x != cx || e.chunk_y != cy || e.chunk_z != cz {
                if e.update_needed {
                    // Entity has left the chunk: chunk.remove(entity, entity.chunkY)
                    if let Some(list) = self.chunks.get_mut(&(e.chunk_x, e.chunk_z)) {
                        list.retain(|&m| m != id);
                    }
                }
                let new_chunk = self.chunks.contains_key(&(cx, cz));
                let e = self.entities.get_mut(&id).unwrap();
                if new_chunk {
                    self.chunks.entry((cx, cz)).or_default().push(id);
                    e.chunk_x = cx; e.chunk_y = cy; e.chunk_z = cz;
                } else {
                    // Entity has left all loaded chunks.
                    e.update_needed = false;
                }
            }
        }
    }
}
