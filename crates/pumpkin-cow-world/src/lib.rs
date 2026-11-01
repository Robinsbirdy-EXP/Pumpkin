#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use bytes::Bytes;
use dashmap::{DashMap, mapref::one::Ref};
use parking_lot::{Mutex, RwLock};
use pumpkin_data::chunk::ChunkStatus;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::{
    chunk::{
        ChunkData, ChunkHeightmaps, ChunkLight, ChunkReadingError, ChunkSections,
        format::{
            anvil::{AnvilRegion, CHUNK_COUNT, REGION_SIZE, SingleChunkDataSerializer},
            linear::LinearV2File,
            pump::PumpFile,
        },
        io::{
            RegionFile,
            converter::{RegionFormat, parse_region_file_name},
        },
        palette::{BiomePalette, BlockPalette},
    },
    level::{CoWBackend, Level, SyncChunk},
    tick::scheduler::ChunkTickScheduler,
};
use rustc_hash::FxHashMap;

/// Alias for a 16x16x16 subchunk block palette.
pub type SubChunk = BlockPalette;

/// Alias for chunk coordinates `(chunk_x, chunk_z)`.
pub type ChunkPos = Vector2<i32>;

/// Global thread-safe registry of immutable base world templates.
pub static TEMPLATES: LazyLock<DashMap<String, Arc<WorldTemplate>>> = LazyLock::new(DashMap::new);

/// Global thread-safe registry of active Copy-on-Write game world instances.
pub static INSTANCES: LazyLock<DashMap<String, Arc<CoWWorldInstance>>> =
    LazyLock::new(DashMap::new);

/// Immutable template representation of a single chunk.
#[derive(Clone)]
pub struct TemplateChunk {
    pub pos: ChunkPos,
    /// Immutable `Arc`-wrapped 16x16x16 subchunk block sections.
    pub sections: Vec<Arc<SubChunk>>,
    /// Immutable `Arc`-wrapped biome sections.
    pub biome_sections: Vec<Arc<BiomePalette>>,
    /// Minimum Y coordinate of the chunk (e.g., `-64`).
    pub min_y: i32,
    /// Initial heightmaps snapshot.
    pub heightmap: ChunkHeightmaps,
    /// Initial lighting snapshot.
    pub light_engine: ChunkLight,
    /// Initial light populated flag.
    pub light_populated: bool,
    /// Chunk status (`ChunkStatus::Full`).
    pub status: ChunkStatus,
    /// Initial block entities serialized as NBT compounds.
    pub block_entities: Arc<FxHashMap<BlockPos, NbtCompound>>,
    /// Optional pre-baked network chunk packet bytes.
    pub cached_packet: Option<Bytes>,
}

impl TemplateChunk {
    #[must_use]
    pub fn new(pos: ChunkPos, num_sections: usize, min_y: i32) -> Self {
        let sections = (0..num_sections)
            .map(|_| Arc::new(SubChunk::default()))
            .collect();
        let biome_sections = (0..num_sections)
            .map(|_| Arc::new(BiomePalette::default()))
            .collect();
        Self {
            pos,
            sections,
            biome_sections,
            min_y,
            heightmap: ChunkHeightmaps::default(),
            light_engine: ChunkLight::default(),
            light_populated: true,
            status: ChunkStatus::Full,
            block_entities: Arc::new(FxHashMap::default()),
            cached_packet: None,
        }
    }

    /// Creates a [`TemplateChunk`] from an existing [`ChunkData`] by cloning only `Arc` section
    /// pointers (zero deep copy of block or biome palettes).
    #[must_use]
    pub fn from_chunk_data(chunk: &ChunkData) -> Self {
        let pos = ChunkPos::new(chunk.x, chunk.z);
        let sections = chunk
            .section
            .block_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect();
        let biome_sections = chunk
            .section
            .biome_sections
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect();
        let heightmap = chunk
            .heightmap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let light_engine = chunk
            .light_engine
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let block_entities = chunk
            .pending_block_entities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

        Self {
            pos,
            sections,
            biome_sections,
            min_y: chunk.section.min_y,
            heightmap,
            light_engine,
            light_populated: chunk.light_populated.load(Ordering::Relaxed),
            status: chunk.status,
            block_entities: Arc::new(block_entities),
            cached_packet: None,
        }
    }
}

/// Active Copy-on-Write chunk inside a [`CoWWorldInstance`].
///
/// Cloning or spawning an [`ActiveChunk`] from a [`TemplateChunk`] only clones `Arc` pointers
/// (`8 bytes` per section) without deep-copying any block or biome palette buffers.
pub struct ActiveChunk {
    pub pos: ChunkPos,
    /// `Arc`-cloned references to the template's chunk sections.
    /// Mutated on demand via `Arc::make_mut`.
    pub sections: Vec<Arc<SubChunk>>,
    /// `Arc`-cloned references to the template's biome sections.
    pub biome_sections: Vec<Arc<BiomePalette>>,
    pub min_y: i32,
    /// True once at least one block or block-entity in this chunk has been mutated.
    pub dirty: bool,
    /// Pre-baked network chunk packet; invalidated (`None`) on first write.
    pub cached_packet: Option<Bytes>,
    /// Block entities in this chunk (`Arc::make_mut` used when mutated).
    pub block_entities: Arc<FxHashMap<BlockPos, NbtCompound>>,
    /// Optional live `ChunkData` reference shared with `pumpkin_world::level::Level` so that
    /// `reset_instant()` can restore sections in-place without recreating the level.
    pub live_chunk: Option<Arc<ChunkData>>,
}

impl ActiveChunk {
    /// Creates an [`ActiveChunk`] from a [`TemplateChunk`] using only `Arc` pointer clones.
    #[must_use]
    pub fn from_template(template: &TemplateChunk) -> Self {
        Self {
            pos: template.pos,
            sections: template.sections.clone(),
            biome_sections: template.biome_sections.clone(),
            min_y: template.min_y,
            dirty: false,
            cached_packet: template.cached_packet.clone(),
            block_entities: Arc::clone(&template.block_entities),
            live_chunk: None,
        }
    }

    #[must_use]
    pub fn empty(pos: ChunkPos, num_sections: usize, min_y: i32) -> Self {
        let sections = (0..num_sections)
            .map(|_| Arc::new(SubChunk::default()))
            .collect();
        let biome_sections = (0..num_sections)
            .map(|_| Arc::new(BiomePalette::default()))
            .collect();
        Self {
            pos,
            sections,
            biome_sections,
            min_y,
            dirty: false,
            cached_packet: None,
            block_entities: Arc::new(FxHashMap::default()),
            live_chunk: None,
        }
    }

    /// Lazily constructs or returns the live [`SyncChunk`] (`Arc<ChunkData>`) sharing the exact
    /// same `Arc<SubChunk>` pointers as `self.sections`.
    pub fn get_or_create_live_chunk(&mut self, template: Option<&TemplateChunk>) -> SyncChunk {
        if let Some(live) = &self.live_chunk {
            return Arc::clone(live);
        }

        let section = ChunkSections::from_arc_palettes(
            self.sections.clone().into_boxed_slice(),
            self.biome_sections.clone().into_boxed_slice(),
            self.min_y,
        );
        let (heightmap, light_engine, light_populated, status) = template.map_or_else(
            || {
                (
                    ChunkHeightmaps::default(),
                    ChunkLight::default(),
                    true,
                    ChunkStatus::Full,
                )
            },
            |tpl| {
                (
                    tpl.heightmap.clone(),
                    tpl.light_engine.clone(),
                    tpl.light_populated,
                    tpl.status,
                )
            },
        );

        let chunk_data = Arc::new(ChunkData {
            section,
            heightmap: std::sync::Mutex::new(heightmap),
            x: self.pos.x,
            z: self.pos.y,
            block_ticks: ChunkTickScheduler::default(),
            fluid_ticks: ChunkTickScheduler::default(),
            pending_block_entities: std::sync::Mutex::new((*self.block_entities).clone()),
            light_engine: std::sync::Mutex::new(light_engine),
            light_populated: AtomicBool::new(light_populated),
            status,
            blending_data: None,
            dirty: AtomicBool::new(self.dirty),
            inhabited_time: AtomicU64::new(0),
            custom_data: std::sync::Mutex::new(NbtCompound::new()),
        });

        self.live_chunk = Some(Arc::clone(&chunk_data));
        chunk_data
    }

    /// Restores this chunk's `Arc` section pointers, block entities, and cached packet
    /// from the given [`TemplateChunk`] without reallocating clean sections.
    pub fn restore_from_template(&mut self, template: &TemplateChunk) {
        self.sections.clone_from(&template.sections);
        self.biome_sections.clone_from(&template.biome_sections);
        self.dirty = false;
        self.cached_packet.clone_from(&template.cached_packet);
        self.block_entities = Arc::clone(&template.block_entities);

        if let Some(live) = &self.live_chunk {
            let (random_tick_sections, randomly_ticking_mask) =
                ChunkSections::build_random_tick_sections_cache(&self.sections);
            *live
                .section
                .block_sections
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                self.sections.clone().into_boxed_slice();
            *live
                .section
                .biome_sections
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                self.biome_sections.clone().into_boxed_slice();
            *live
                .section
                .random_tick_sections
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = random_tick_sections;
            live.section
                .randomly_ticking_mask
                .store(randomly_ticking_mask, Ordering::Relaxed);
            *live
                .heightmap
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = template.heightmap.clone();
            live.pending_block_entities
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone_from(&template.block_entities);
            live.dirty.store(false, Ordering::Relaxed);
        }
    }
}

/// Immutable base world template stored in [`TEMPLATES`].
pub struct WorldTemplate {
    pub name: String,
    pub source_path: Option<PathBuf>,
    pub min_y: i32,
    pub num_sections: usize,
    pub chunks: DashMap<ChunkPos, TemplateChunk>,
}

impl WorldTemplate {
    #[must_use]
    pub fn new(name: impl Into<String>, num_sections: usize, min_y: i32) -> Self {
        Self {
            name: name.into(),
            source_path: None,
            min_y,
            num_sections,
            chunks: DashMap::new(),
        }
    }

    /// Inserts a [`TemplateChunk`] into this template.
    pub fn insert_chunk(&self, chunk: TemplateChunk) {
        self.chunks.insert(chunk.pos, chunk);
    }
}

/// Active Copy-on-Write world instance backed by a [`WorldTemplate`].
pub struct CoWWorldInstance {
    pub instance_id: String,
    pub template_name: String,
    pub template: Arc<WorldTemplate>,
    pub chunks: DashMap<ChunkPos, ActiveChunk>,
    pub dirty_chunks: Mutex<HashSet<ChunkPos>>,
    /// Optional reference to the bound `pumpkin_world::level::Level`.
    pub bound_level: RwLock<Option<Arc<Level>>>,
}

impl std::fmt::Debug for TemplateChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplateChunk")
            .field("pos", &self.pos)
            .field("min_y", &self.min_y)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ActiveChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveChunk")
            .field("pos", &self.pos)
            .field("dirty", &self.dirty)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for WorldTemplate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldTemplate")
            .field("name", &self.name)
            .field("num_sections", &self.num_sections)
            .field("min_y", &self.min_y)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for CoWWorldInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoWWorldInstance")
            .field("instance_id", &self.instance_id)
            .field("template_name", &self.template_name)
            .finish_non_exhaustive()
    }
}

impl CoWWorldInstance {
    /// Spawns a new [`CoWWorldInstance`] from a registered template in [`TEMPLATES`].
    ///
    /// Clones only the `Arc` pointers from the template into a new `DashMap` (`O(N_chunks)`
    /// pointer copies, zero deep allocations).
    pub fn spawn_from_template(
        template_name: &str,
        instance_id: &str,
    ) -> Result<Arc<Self>, String> {
        let template = TEMPLATES
            .get(template_name)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| format!("Template '{template_name}' is not loaded in TEMPLATES"))?;

        let chunks = DashMap::with_capacity(template.chunks.len());
        for entry in &template.chunks {
            chunks.insert(*entry.key(), ActiveChunk::from_template(entry.value()));
        }

        let instance = Arc::new(Self {
            instance_id: instance_id.to_owned(),
            template_name: template_name.to_owned(),
            template,
            chunks,
            dirty_chunks: Mutex::new(HashSet::new()),
            bound_level: RwLock::new(None),
        });

        INSTANCES.insert(instance_id.to_owned(), Arc::clone(&instance));
        Ok(instance)
    }

    /// Binds this `CoW` instance to a [`Level`] so the level bypasses disk I/O and routes block
    /// mutations through Copy-on-Write tracking.
    pub fn bind_to_level(self: &Arc<Self>, level: &Arc<Level>) {
        *self.bound_level.write() = Some(Arc::clone(level));
        let backend: Arc<dyn CoWBackend> = Arc::clone(self) as Arc<dyn CoWBackend>;
        level.set_cow_backend(Some(backend));
    }

    /// Returns read-only access to the [`ActiveChunk`] at `pos`.
    #[must_use]
    pub fn get_chunk(&self, pos: ChunkPos) -> Option<Ref<'_, ChunkPos, ActiveChunk>> {
        self.chunks.get(&pos)
    }

    /// Mutates a specific subchunk section at `(pos, section_idx)` using `Arc::make_mut`.
    ///
    /// Records `pos` in `dirty_chunks`, marks the chunk as `dirty`, invalidates `cached_packet`,
    /// and clones the underlying [`SubChunk`] memory ONLY on the first write to that section.
    pub fn mutate_chunk_section<F>(&self, pos: ChunkPos, section_idx: usize, mutator: F) -> bool
    where
        F: FnOnce(&mut SubChunk),
    {
        if !self.chunks.contains_key(&pos)
            && let Some(template_chunk) = self.template.chunks.get(&pos)
        {
            self.chunks
                .insert(pos, ActiveChunk::from_template(template_chunk.value()));
        }

        let Some(mut chunk_entry) = self.chunks.get_mut(&pos) else {
            return false;
        };

        let chunk = chunk_entry.value_mut();
        let Some(section_arc) = chunk.sections.get_mut(section_idx) else {
            return false;
        };

        chunk.dirty = true;
        chunk.cached_packet = None;
        let section_mut = Arc::make_mut(section_arc);
        mutator(section_mut);
        let updated_arc = Arc::clone(section_arc);

        if let Some(live) = &chunk.live_chunk
            && let Some(live_sec) = live
                .section
                .block_sections
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_mut(section_idx)
        {
            *live_sec = updated_arc;
        }
        drop(chunk_entry);

        self.dirty_chunks.lock().insert(pos);
        true
    }

    /// Instantly resets all mutated chunks in this instance back to their immutable template state.
    ///
    /// Locks `dirty_chunks`, iterates ONLY over mutated chunk coordinates, restores their `Arc`
    /// pointers from `self.template`, and returns the list of reset [`ChunkPos`] so the server
    /// can resend those chunks to nearby players.
    #[must_use]
    pub fn reset_instant(&self) -> Vec<ChunkPos> {
        let mut dirty_guard = self.dirty_chunks.lock();
        let reset_positions: Vec<ChunkPos> = dirty_guard.drain().collect();
        drop(dirty_guard);

        for &pos in &reset_positions {
            if let Some(template_chunk) = self.template.chunks.get(&pos) {
                if let Some(mut active_chunk) = self.chunks.get_mut(&pos) {
                    active_chunk.restore_from_template(template_chunk.value());
                }
            } else if let Some(mut active_chunk) = self.chunks.get_mut(&pos) {
                let empty_tpl =
                    TemplateChunk::new(pos, self.template.num_sections, self.template.min_y);
                active_chunk.restore_from_template(&empty_tpl);
            }
        }

        reset_positions
    }

    /// Returns `(total_chunks, dirty_chunks_count)` for diagnostics/stats.
    #[must_use]
    pub fn stats(&self) -> (usize, usize) {
        (self.chunks.len(), self.dirty_chunks.lock().len())
    }
}

impl CoWBackend for CoWWorldInstance {
    fn fetch_chunk(&self, pos: Vector2<i32>) -> Option<SyncChunk> {
        if let Some(mut active) = self.chunks.get_mut(&pos) {
            let tpl = self.template.chunks.get(&pos);
            return Some(active.get_or_create_live_chunk(tpl.as_deref()));
        }

        if let Some(tpl) = self.template.chunks.get(&pos) {
            let mut active = ActiveChunk::from_template(tpl.value());
            let live = active.get_or_create_live_chunk(Some(tpl.value()));
            self.chunks.insert(pos, active);
            return Some(live);
        }

        let mut active = ActiveChunk::empty(pos, self.template.num_sections, self.template.min_y);
        let live = active.get_or_create_live_chunk(None);
        self.chunks.insert(pos, active);
        Some(live)
    }

    fn on_chunk_mutated(&self, pos: Vector2<i32>, section_idx: usize, chunk: &SyncChunk) {
        if let Some(mut active) = self.chunks.get_mut(&pos) {
            active.dirty = true;
            active.cached_packet = None;
            if let Some(sec) = chunk
                .section
                .block_sections
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(section_idx)
                && let Some(active_sec) = active.sections.get_mut(section_idx)
            {
                *active_sec = Arc::clone(sec);
            }
        }
        self.dirty_chunks.lock().insert(pos);
    }
}

/// Registers a [`WorldTemplate`] in [`TEMPLATES`].
pub fn register_template(template: WorldTemplate) -> Arc<WorldTemplate> {
    let name = template.name.clone();
    let arc = Arc::new(template);
    TEMPLATES.insert(name, Arc::clone(&arc));
    arc
}

/// Destroys an active [`CoWWorldInstance`] by removing it from [`INSTANCES`].
pub fn destroy_instance(instance_id: &str) -> Result<Arc<CoWWorldInstance>, String> {
    INSTANCES
        .remove(instance_id)
        .map(|(_, instance)| instance)
        .ok_or_else(|| format!("CoW instance '{instance_id}' does not exist"))
}

fn read_region_chunks<R: RegionFile>(
    path: &Path,
    region: Vector2<i32>,
    template: &WorldTemplate,
) -> Result<usize, ChunkReadingError> {
    let file = R::open(path, region)?;
    let mut loaded = 0usize;
    for index in 0..CHUNK_COUNT {
        if let Some(raw) = file.read_chunk(index)? {
            let chunk_pos = Vector2::new(
                region.x * REGION_SIZE as i32 + (index % REGION_SIZE) as i32,
                region.y * REGION_SIZE as i32 + (index / REGION_SIZE) as i32,
            );
            let bytes = R::decode_chunk(raw)?;
            if let Ok(chunk_data) = ChunkData::from_bytes(&bytes, chunk_pos) {
                template.insert_chunk(TemplateChunk::from_chunk_data(&chunk_data));
                loaded += 1;
            }
        }
    }
    Ok(loaded)
}

/// Loads all region files from a region directory on disk into `template`.
pub fn ingest_region_folder_into_template(
    region_folder: &Path,
    template: &WorldTemplate,
) -> Result<usize, String> {
    if !region_folder.exists() {
        return Ok(0);
    }
    let entries = std::fs::read_dir(region_folder).map_err(|e| {
        format!(
            "Failed reading region folder {}: {e}",
            region_folder.display()
        )
    })?;

    let mut total_chunks = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((region_pos, format)) = parse_region_file_name(file_name) else {
            continue;
        };
        let count_res = match format {
            RegionFormat::Anvil => read_region_chunks::<AnvilRegion>(&path, region_pos, template),
            RegionFormat::Linear => read_region_chunks::<LinearV2File>(&path, region_pos, template),
            RegionFormat::Pump => read_region_chunks::<PumpFile>(&path, region_pos, template),
        };
        if let Ok(count) = count_res {
            total_chunks += count;
        }
    }
    Ok(total_chunks)
}

/// Ingests a standard loaded [`Level`] (both its disk region files and currently loaded in-memory
/// chunks) into [`TEMPLATES`] under `template_name`.
pub fn ingest_level_into_templates(
    template_name: &str,
    level: &Level,
) -> Result<Arc<WorldTemplate>, String> {
    let world_gen = level.world_gen.load();
    let dim = world_gen.dimension();
    let num_sections = (dim.height as usize) / BlockPalette::SIZE;
    let mut template = WorldTemplate::new(template_name, num_sections, dim.min_y);
    template.source_path = Some(level.level_folder.root_folder.clone());

    let _ = ingest_region_folder_into_template(&level.level_folder.region_folder, &template)?;

    for entry in level.loaded_chunks.iter() {
        template.insert_chunk(TemplateChunk::from_chunk_data(entry.value()));
    }

    Ok(register_template(template))
}

/// Loads a world folder from disk into [`TEMPLATES`] under `template_name`.
pub fn load_template_from_disk(
    template_name: &str,
    root_folder: PathBuf,
) -> Result<Arc<WorldTemplate>, String> {
    let mut template = WorldTemplate::new(template_name, 24, -64);

    let candidate_region_dirs = [
        root_folder
            .join("dimensions")
            .join("minecraft")
            .join("overworld")
            .join("region"),
        root_folder.join("region"),
    ];
    template.source_path = Some(root_folder);

    for region_dir in &candidate_region_dirs {
        if region_dir.exists() {
            let _ = ingest_region_folder_into_template(region_dir, &template)?;
        }
    }

    Ok(register_template(template))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::Block;

    #[test]
    fn cow_zero_copy_spawn_mutate_and_instant_reset() {
        let template_name = "test_arena_template";
        let template = WorldTemplate::new(template_name, 24, -64);

        let target_pos = ChunkPos::new(0, 0);
        let neighbor_pos = ChunkPos::new(1, 0);
        template.insert_chunk(TemplateChunk::new(target_pos, 24, -64));
        template.insert_chunk(TemplateChunk::new(neighbor_pos, 24, -64));

        let template_arc = register_template(template);

        let template_sec4_ptr = Arc::as_ptr(
            &template_arc
                .chunks
                .get(&target_pos)
                .expect("target chunk in template")
                .sections[4],
        );
        let template_sec5_ptr = Arc::as_ptr(
            &template_arc
                .chunks
                .get(&target_pos)
                .expect("target chunk in template")
                .sections[5],
        );

        // 1. Spawn 10 instances and verify every untouched section shares the exact same memory address.
        let mut instances = Vec::with_capacity(10);
        for i in 1..=10 {
            let instance_id = format!("arena_inst_{i}");
            let inst = CoWWorldInstance::spawn_from_template(template_name, &instance_id)
                .expect("spawn instance");
            let chunk_ref = inst.get_chunk(target_pos).expect("chunk exists");
            for sec_idx in 0..24 {
                let expected_ptr = Arc::as_ptr(
                    &template_arc
                        .chunks
                        .get(&target_pos)
                        .expect("template chunk")
                        .sections[sec_idx],
                );
                assert_eq!(
                    Arc::as_ptr(&chunk_ref.sections[sec_idx]),
                    expected_ptr,
                    "instance #{i} section {sec_idx} must share exact template pointer"
                );
            }
            drop(chunk_ref);
            instances.push(inst);
        }

        // 2. Mutate 1 block in instance #1 (section 4) and verify ONLY section 4 in instance #1 changes pointer.
        let stone_id = Block::STONE.default_state.id;
        let mutated = instances[0].mutate_chunk_section(target_pos, 4, |subchunk| {
            subchunk.set(2, 3, 4, stone_id);
        });
        assert!(mutated, "mutation should succeed");

        let inst1_chunk = instances[0].get_chunk(target_pos).expect("inst1 chunk");
        assert!(inst1_chunk.dirty, "inst1 target chunk must be marked dirty");
        assert_ne!(
            Arc::as_ptr(&inst1_chunk.sections[4]),
            template_sec4_ptr,
            "mutated section 4 in instance #1 must have a new CoW allocation pointer"
        );
        assert_eq!(
            inst1_chunk.sections[4].get(2, 3, 4),
            stone_id,
            "mutated block state must be STONE in instance #1"
        );
        assert_eq!(
            Arc::as_ptr(&inst1_chunk.sections[5]),
            template_sec5_ptr,
            "untouched section 5 in instance #1 must still share the template pointer"
        );
        drop(inst1_chunk);

        // Instances #2..#10 must still keep the exact original template pointer for section 4.
        for (idx, inst) in instances.iter().enumerate().skip(1) {
            let chunk_ref = inst.get_chunk(target_pos).expect("chunk exists");
            assert_eq!(
                Arc::as_ptr(&chunk_ref.sections[4]),
                template_sec4_ptr,
                "instance #{} must still share the original template pointer",
                idx + 1
            );
            assert_eq!(
                chunk_ref.sections[4].get(2, 3, 4),
                Block::AIR.default_state.id,
                "instance #{} block must remain AIR",
                idx + 1
            );
        }

        // 3. Calling reset_instant() on instance #1 restores the original template pointer.
        let reset_chunks = instances[0].reset_instant();
        assert_eq!(reset_chunks, vec![target_pos]);

        let inst1_chunk_after = instances[0].get_chunk(target_pos).expect("inst1 chunk");
        assert!(
            !inst1_chunk_after.dirty,
            "dirty flag must be cleared on reset"
        );
        assert_eq!(
            Arc::as_ptr(&inst1_chunk_after.sections[4]),
            template_sec4_ptr,
            "reset_instant() must restore the exact original template Arc pointer"
        );
        assert_eq!(
            inst1_chunk_after.sections[4].get(2, 3, 4),
            Block::AIR.default_state.id,
            "block state must be restored to template state"
        );
    }
}
