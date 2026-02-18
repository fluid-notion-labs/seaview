//! Headless debug loader for sequence analysis without GPU/window.
//!
//! Discovers sequences, parses mesh files directly (GLB/glTF/STL/OBJ), computes
//! memory estimates, applies budget logic, and reports metrics.
//!
//! Usage:
//!     cargo run --bin debug_loader --release -- <directory> [OPTIONS]
//!
//! Options:
//!     --sample N       Number of frames to sample for size estimation (default: 10)
//!     --budget-mb N    Manual memory budget override in MB
//!     --all            Parse every frame (slow but accurate)
//!     --json           Output as JSON instead of human-readable

use std::path::{Path, PathBuf};
use std::time::Instant;

use nvml_wrapper::Nvml;
use tobj;

use seaview::{
    discover_sequences, estimate_mesh_gpu_bytes, human_bytes, GlobalSettings, MemoryBudget,
    MemorySettings, MeshFileFormat, NoVramDetector, NvmlBackend, RamDetector, Sequence,
    SequencePatterns, SysInfoBackend,
};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

struct Args {
    directory: PathBuf,
    sample_count: usize,
    budget_mb: Option<u64>,
    parse_all: bool,
    json_output: bool,
}

fn parse_args() -> Args {
    let mut args = std::env::args().skip(1);
    let mut directory = None;
    let mut sample_count = 10;
    let mut budget_mb = None;
    let mut parse_all = false;
    let mut json_output = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sample" => {
                sample_count = args
                    .next()
                    .expect("--sample requires a number")
                    .parse()
                    .expect("--sample must be a positive integer");
            }
            "--budget-mb" => {
                budget_mb = Some(
                    args.next()
                        .expect("--budget-mb requires a number")
                        .parse()
                        .expect("--budget-mb must be a positive integer"),
                );
            }
            "--all" => parse_all = true,
            "--json" => json_output = true,
            "--help" | "-h" => {
                eprintln!("Usage: debug_loader <directory> [OPTIONS]");
                eprintln!();
                eprintln!("Options:");
                eprintln!("  --sample N      Frames to sample for size estimation (default: 10)");
                eprintln!("  --budget-mb N   Manual memory budget in MB");
                eprintln!("  --all           Parse every frame (slow)");
                eprintln!("  --json          JSON output");
                std::process::exit(0);
            }
            other => {
                if other.starts_with('-') {
                    eprintln!("Unknown option: {}", other);
                    std::process::exit(1);
                }
                directory = Some(PathBuf::from(other));
            }
        }
    }

    let directory = directory.unwrap_or_else(|| {
        eprintln!("Error: no directory specified. Run with --help for usage.");
        std::process::exit(1);
    });

    Args {
        directory,
        sample_count,
        budget_mb,
        parse_all,
        json_output,
    }
}

// ---------------------------------------------------------------------------
// Helpers for library types
// ---------------------------------------------------------------------------

/// Detect the mesh format of a sequence from its first frame's extension.
fn detect_format(seq: &Sequence) -> MeshFileFormat {
    seq.frames
        .first()
        .and_then(|f| f.path.extension())
        .and_then(|e| e.to_str())
        .and_then(MeshFileFormat::from_extension)
        .unwrap_or(MeshFileFormat::Glb)
}

// ---------------------------------------------------------------------------
// Mesh parsing (headless, no GPU)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct MeshStats {
    vertex_count: usize,
    index_count: usize,
    /// File size on disk
    file_bytes: u64,
    /// Estimated GPU memory
    estimated_gpu_bytes: u64,
    /// Time to parse in microseconds
    parse_us: u64,
}

fn parse_glb_stats(path: &Path) -> Result<MeshStats, String> {
    let t0 = Instant::now();
    let file_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let (document, buffers, _) =
        gltf::import(path).map_err(|e| format!("gltf import error: {}", e))?;

    let mut total_vertices = 0usize;
    let mut total_indices = 0usize;

    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            if let Some(positions) = primitive.get(&gltf::Semantic::Positions) {
                total_vertices += positions.count();
            }
            if let Some(accessor) = primitive.indices() {
                total_indices += accessor.count();
            }
        }
    }

    // If we got zero from accessors, try reading buffer data directly
    if total_vertices == 0 && !buffers.is_empty() {
        for mesh in document.meshes() {
            for primitive in mesh.primitives() {
                if let Some(pos_accessor) = primitive.get(&gltf::Semantic::Positions) {
                    total_vertices += pos_accessor.count();
                }
            }
        }
    }

    let parse_us = t0.elapsed().as_micros() as u64;

    Ok(MeshStats {
        vertex_count: total_vertices,
        index_count: total_indices,
        file_bytes,
        estimated_gpu_bytes: estimate_mesh_gpu_bytes(total_vertices, total_indices),
        parse_us,
    })
}

fn parse_stl_stats(path: &Path) -> Result<MeshStats, String> {
    let t0 = Instant::now();
    let file_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let mut file = std::fs::File::open(path).map_err(|e| format!("open error: {}", e))?;
    let mut reader = std::io::BufReader::new(&mut file);
    let stl = stl_io::read_stl(&mut reader).map_err(|e| format!("STL parse error: {}", e))?;

    let triangle_count = stl.faces.len();
    // STL: 3 vertices per triangle (no sharing), 3 indices per triangle
    let vertex_count = triangle_count * 3;
    let index_count = triangle_count * 3;

    let parse_us = t0.elapsed().as_micros() as u64;

    Ok(MeshStats {
        vertex_count,
        index_count,
        file_bytes,
        estimated_gpu_bytes: estimate_mesh_gpu_bytes(vertex_count, index_count),
        parse_us,
    })
}

fn parse_obj_stats(path: &Path) -> Result<MeshStats, String> {
    let t0 = Instant::now();
    let file_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let (models, _materials) = tobj::load_obj(
        path,
        &tobj::LoadOptions {
            triangulate: true,
            single_index: true,
            ..Default::default()
        },
    )
    .map_err(|e| format!("OBJ parse error: {}", e))?;

    let vertex_count: usize = models.iter().map(|m| m.mesh.positions.len() / 3).sum();
    let index_count: usize = models.iter().map(|m| m.mesh.indices.len()).sum();

    let parse_us = t0.elapsed().as_micros() as u64;

    Ok(MeshStats {
        vertex_count,
        index_count,
        file_bytes,
        estimated_gpu_bytes: estimate_mesh_gpu_bytes(vertex_count, index_count),
        parse_us,
    })
}

fn parse_frame(path: &Path, format: MeshFileFormat) -> Result<MeshStats, String> {
    match format {
        MeshFileFormat::Glb | MeshFileFormat::Gltf => parse_glb_stats(path),
        MeshFileFormat::Stl => parse_stl_stats(path),
        MeshFileFormat::Obj => parse_obj_stats(path),
    }
}

// ---------------------------------------------------------------------------
// Settings loading (no Bevy)
// ---------------------------------------------------------------------------

fn load_local_settings(directory: &Path) -> Option<String> {
    let path = directory.join("seaview.toml");
    if path.exists() {
        std::fs::read_to_string(&path).ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// VRAM detection — multi-GPU enumeration for display purposes.
// Budget computation uses lib's NvmlBackend (device 0 only).
// ---------------------------------------------------------------------------

struct GpuInfo {
    device_name: String,
    total_vram: u64,
    free_vram: u64,
}

fn detect_gpus() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    let nvml = match Nvml::init() {
        Ok(n) => n,
        Err(_) => return gpus,
    };
    let count = match nvml.device_count() {
        Ok(c) => c,
        Err(_) => return gpus,
    };
    for i in 0..count {
        if let Ok(device) = nvml.device_by_index(i) {
            let name = device.name().unwrap_or_else(|_| format!("GPU {}", i));
            if let Ok(mem) = device.memory_info() {
                gpus.push(GpuInfo {
                    device_name: name,
                    total_vram: mem.total,
                    free_vram: mem.free,
                });
            }
        }
    }
    gpus
}

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// JSON output
// ---------------------------------------------------------------------------

fn print_json(
    seq: &Sequence,
    format: MeshFileFormat,
    sampled: &[MeshStats],
    budget: &MemoryBudget,
    mem_settings: &MemorySettings,
    load_section_start: usize,
    load_section_end: usize,
    local_toml: bool,
    global_toml: bool,
    total_parse_ms: f64,
) {
    let gb = 1_073_741_824.0_f64;

    // Minimal JSON without serde_json — keep deps light
    println!("{{");
    println!("  \"sequence\": {{");
    println!("    \"name\": {:?},", seq.name);
    println!("    \"format\": {:?},", format.label());
    println!("    \"total_frames\": {},", seq.frames.len());
    println!(
        "    \"total_file_size_bytes\": {},",
        seq.frames.iter().map(|f| f.file_size).sum::<u64>()
    );
    println!("    \"frames_sampled\": {}", sampled.len());
    println!("  }},");

    if !sampled.is_empty() {
        let avg_verts = sampled.iter().map(|s| s.vertex_count).sum::<usize>() / sampled.len();
        let avg_indices = sampled.iter().map(|s| s.index_count).sum::<usize>() / sampled.len();
        let avg_gpu =
            sampled.iter().map(|s| s.estimated_gpu_bytes).sum::<u64>() / sampled.len() as u64;
        let min_gpu = sampled
            .iter()
            .map(|s| s.estimated_gpu_bytes)
            .min()
            .unwrap_or(0);
        let max_gpu = sampled
            .iter()
            .map(|s| s.estimated_gpu_bytes)
            .max()
            .unwrap_or(0);
        let avg_file = sampled.iter().map(|s| s.file_bytes).sum::<u64>() / sampled.len() as u64;
        let avg_parse = sampled.iter().map(|s| s.parse_us).sum::<u64>() / sampled.len() as u64;

        println!("  \"mesh_stats\": {{");
        println!("    \"avg_vertices\": {},", avg_verts);
        println!("    \"avg_indices\": {},", avg_indices);
        println!("    \"avg_gpu_bytes\": {},", avg_gpu);
        println!("    \"min_gpu_bytes\": {},", min_gpu);
        println!("    \"max_gpu_bytes\": {},", max_gpu);
        println!("    \"avg_file_bytes\": {},", avg_file);
        println!("    \"avg_parse_us\": {}", avg_parse);
        println!("  }},");
    }

    println!("  \"budget\": {{");
    println!("    \"ram_total_gb\": {:.2},", budget.total_ram as f64 / gb);
    println!(
        "    \"ram_available_gb\": {:.2},",
        budget.available_ram as f64 / gb
    );
    println!(
        "    \"ram_budget_gb\": {:.2},",
        budget.ram_budget as f64 / gb
    );
    println!("    \"ram_fraction\": {},", mem_settings.ram_fraction);
    if let Some(vt) = budget.total_vram {
        println!("    \"vram_total_bytes\": {},", vt);
    }
    println!("    \"vram_budget_bytes\": {},", budget.vram_budget);
    println!("    \"gpu_fraction\": {},", mem_settings.gpu_fraction);
    println!(
        "    \"effective_budget_bytes\": {},",
        budget.effective_budget()
    );
    println!("    \"frame_bytes\": {},", budget.frame_size_estimate);
    println!("    \"max_frames\": {}", budget.max_frames);
    println!("  }},");

    println!("  \"load_section\": {{");
    println!("    \"start\": {},", load_section_start);
    println!("    \"end\": {},", load_section_end);
    println!(
        "    \"frame_count\": {}",
        load_section_end.saturating_sub(load_section_start)
    );
    println!("  }},");

    println!("  \"settings\": {{");
    println!("    \"local_toml\": {},", local_toml);
    println!("    \"global_toml\": {}", global_toml);
    println!("  }},");

    println!("  \"timing\": {{");
    println!("    \"total_parse_ms\": {:.1}", total_parse_ms);
    println!("  }}");
    println!("}}");
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args = parse_args();
    let dir = &args.directory;

    if !dir.is_dir() {
        eprintln!("Error: {:?} is not a directory", dir);
        std::process::exit(1);
    }

    let human = !args.json_output;

    // ── Settings ──────────────────────────────────────────────────────────
    let local_toml_str = load_local_settings(dir);
    let global_settings = GlobalSettings::load().unwrap_or_default();
    let has_global_toml = global_settings.memory.is_some() || global_settings.defaults.is_some();
    let mem_settings = global_settings.memory.clone().unwrap_or_default();
    let ram_fraction = mem_settings.ram_fraction;
    let gpu_fraction = mem_settings.gpu_fraction;

    // Extract load_section from local config (best effort)
    let (cfg_ls_start, cfg_ls_end) = local_toml_str
        .as_deref()
        .and_then(|s| {
            let table: toml::Table = toml::from_str(s).ok()?;
            let ls = table.get("load_section")?.as_table()?;
            let start = ls.get("start")?.as_integer()? as usize;
            let end = ls.get("end")?.as_integer()? as usize;
            Some((start, end))
        })
        .unwrap_or((0, usize::MAX));

    if human {
        println!("╔══════════════════════════════════════════════════════════════╗");
        println!("║              Seaview Debug Loader — Headless                ║");
        println!("╚══════════════════════════════════════════════════════════════╝");
        println!();
        println!("Directory:     {:?}", dir);
        println!(
            "Local config:  {}",
            if local_toml_str.is_some() {
                "seaview.toml found ✓"
            } else {
                "none"
            }
        );
        println!(
            "Global config: {}",
            if has_global_toml { "found ✓" } else { "none" }
        );
        if cfg_ls_start != 0 || cfg_ls_end != usize::MAX {
            println!("Config load_section: [{}..{})", cfg_ls_start, cfg_ls_end);
        }
        println!();
    }

    // ── Discovery ─────────────────────────────────────────────────────────
    let t_discover = Instant::now();
    let patterns = SequencePatterns::default();
    let orientation = seaview::coordinates::SourceOrientation::default();
    let sequences = match discover_sequences(dir, false, &patterns, orientation) {
        Ok(seqs) => seqs,
        Err(e) => {
            eprintln!("Error discovering sequences: {}", e);
            std::process::exit(1);
        }
    };
    let discover_ms = t_discover.elapsed().as_secs_f64() * 1000.0;

    if sequences.is_empty() {
        if human {
            println!("No sequences found in {:?}", dir);
        }
        std::process::exit(0);
    }

    if human {
        println!(
            "Discovery: {} sequence(s) in {:.1} ms",
            sequences.len(),
            discover_ms
        );
        println!();
    }

    // ── Hardware detection (via lib) ──────────────────────────────────────
    let ram_backend = SysInfoBackend::new();
    let nvml_backend = NvmlBackend::new(); // Option<NvmlBackend>

    if human {
        println!(
            "RAM: {:.1} GB total, {:.1} GB available",
            ram_backend.total_ram_bytes() as f64 / 1_073_741_824.0,
            ram_backend.available_ram_bytes() as f64 / 1_073_741_824.0,
        );
        println!(
            "     fraction={:.0}% → budget {:.1} GB",
            ram_fraction * 100.0,
            ram_backend.total_ram_bytes() as f64 * ram_fraction as f64 / 1_073_741_824.0,
        );
        println!();
    }

    // ── GPU / VRAM (multi-GPU enumeration for display) ────────────────────
    let gpus = detect_gpus();
    let primary_gpu = gpus.first();

    if human {
        if gpus.is_empty() {
            println!("GPU: NVML unavailable — no VRAM detection");
        } else {
            for (i, gpu) in gpus.iter().enumerate() {
                let marker = if i == 0 { "→" } else { " " };
                println!(
                    "  {} GPU {}: {} — {:.1} GB total, {:.1} GB free",
                    marker,
                    i,
                    gpu.device_name,
                    gpu.total_vram as f64 / 1_073_741_824.0,
                    gpu.free_vram as f64 / 1_073_741_824.0,
                );
            }
            if let Some(gpu) = primary_gpu {
                println!(
                    "     fraction={:.0}% → VRAM budget {:.1} GB",
                    gpu_fraction * 100.0,
                    gpu.total_vram as f64 * gpu_fraction as f64 / 1_073_741_824.0,
                );
            }
        }
        println!();
    }

    // ── Per-sequence analysis ─────────────────────────────────────────────
    for seq in &sequences {
        let n = seq.frames.len();
        let total_file_bytes: u64 = seq.frames.iter().map(|f| f.file_size).sum();

        let format = detect_format(seq);

        if human {
            println!("━━━ Sequence: \"{}\" ━━━", seq.name);
            println!("  Format:       {}", format.label());
            println!("  Frames:       {}", n);
            println!(
                "  File range:   {} → {}",
                seq.frames.first().map(|f| f.frame_number).unwrap_or(0),
                seq.frames.last().map(|f| f.frame_number).unwrap_or(0),
            );
            println!("  Total on disk: {}", human_bytes(total_file_bytes));
            println!();
        }

        // ── Sample frames ─────────────────────────────────────────────────
        let indices_to_sample: Vec<usize> = if args.parse_all {
            (0..n).collect()
        } else {
            // Evenly spaced sample: first, last, and evenly distributed in between
            let count = args.sample_count.min(n);
            if count <= 2 {
                if n == 1 {
                    vec![0]
                } else {
                    vec![0, n - 1]
                }
            } else {
                let mut indices = Vec::with_capacity(count);
                for i in 0..count {
                    let idx = if count == 1 {
                        0
                    } else {
                        i * (n - 1) / (count - 1)
                    };
                    indices.push(idx);
                }
                indices.sort();
                indices.dedup();
                indices
            }
        };

        if human {
            println!("  Sampling {} of {} frames...", indices_to_sample.len(), n);
        }

        let mut sampled: Vec<MeshStats> = Vec::new();
        let mut errors: Vec<(usize, String)> = Vec::new();
        let t_parse_start = Instant::now();

        for &idx in &indices_to_sample {
            let frame = &seq.frames[idx];
            match parse_frame(&frame.path, format) {
                Ok(stats) => {
                    if human && sampled.len() < 3 {
                        println!(
                            "    frame {:>5}: {:>7} verts, {:>7} indices, gpu ≈ {:>8}, disk {:>8}, parse {:>6} µs",
                            frame.frame_number,
                            stats.vertex_count,
                            stats.index_count,
                            human_bytes(stats.estimated_gpu_bytes),
                            human_bytes(stats.file_bytes),
                            stats.parse_us,
                        );
                    }
                    sampled.push(stats);
                }
                Err(e) => {
                    errors.push((frame.frame_number, e));
                }
            }
        }

        let total_parse_ms = t_parse_start.elapsed().as_secs_f64() * 1000.0;

        if !errors.is_empty() && human {
            println!("    ⚠ {} parse errors:", errors.len());
            for (frame_num, err) in errors.iter().take(3) {
                println!("      frame {}: {}", frame_num, err);
            }
            if errors.len() > 3 {
                println!("      ... and {} more", errors.len() - 3);
            }
        }

        if sampled.is_empty() {
            if human {
                println!("    No frames parsed successfully — skipping budget analysis.");
            }
            println!();
            continue;
        }

        // ── Statistics ────────────────────────────────────────────────────
        let avg_verts = sampled.iter().map(|s| s.vertex_count).sum::<usize>() / sampled.len();
        let avg_indices = sampled.iter().map(|s| s.index_count).sum::<usize>() / sampled.len();
        let avg_gpu =
            sampled.iter().map(|s| s.estimated_gpu_bytes).sum::<u64>() / sampled.len() as u64;
        let min_gpu = sampled
            .iter()
            .map(|s| s.estimated_gpu_bytes)
            .min()
            .unwrap_or(0);
        let max_gpu = sampled
            .iter()
            .map(|s| s.estimated_gpu_bytes)
            .max()
            .unwrap_or(0);
        let avg_file = sampled.iter().map(|s| s.file_bytes).sum::<u64>() / sampled.len() as u64;
        let avg_parse_us = sampled.iter().map(|s| s.parse_us).sum::<u64>() / sampled.len() as u64;

        let variance_factor = if min_gpu > 0 {
            max_gpu as f64 / min_gpu as f64
        } else {
            0.0
        };

        if human {
            if sampled.len() > 3 {
                println!("    ... ({} more sampled)", sampled.len() - 3);
            }
            println!();
            println!(
                "  ┌─ Mesh Statistics (from {} samples) ─────────────────────┐",
                sampled.len()
            );
            println!(
                "  │  Avg vertices:    {:>10}                           │",
                avg_verts
            );
            println!(
                "  │  Avg indices:     {:>10}                           │",
                avg_indices
            );
            println!(
                "  │  Avg GPU memory:  {:>10}                           │",
                human_bytes(avg_gpu)
            );
            println!(
                "  │  Min GPU memory:  {:>10}                           │",
                human_bytes(min_gpu)
            );
            println!(
                "  │  Max GPU memory:  {:>10}  (variance: {:.1}x)       │",
                human_bytes(max_gpu),
                variance_factor
            );
            println!(
                "  │  Avg file size:   {:>10}                           │",
                human_bytes(avg_file)
            );
            println!(
                "  │  Avg parse time:  {:>7} µs                          │",
                avg_parse_us
            );
            println!("  └──────────────────────────────────────────────────────────┘");
            println!();
        }

        // ── Budget (via lib) ──────────────────────────────────────────────
        let mut budget = if let Some(mb) = args.budget_mb {
            MemoryBudget::with_manual_budget(mb)
        } else {
            match &nvml_backend {
                Some(nvml) => MemoryBudget::detect(&ram_backend, nvml, &mem_settings),
                None => MemoryBudget::detect(&ram_backend, &NoVramDetector, &mem_settings),
            }
        };
        budget.update_frame_estimate(avg_gpu);

        // ── Load section ──────────────────────────────────────────────────
        let mut ls_start = cfg_ls_start;
        let mut ls_end = cfg_ls_end.min(n);
        ls_start = ls_start.min(ls_end);
        let ls_count = ls_end.saturating_sub(ls_start);
        if ls_count > budget.max_frames {
            ls_end = ls_start + budget.max_frames;
        }
        let ls_count = ls_end.saturating_sub(ls_start);

        let total_gpu_estimate = avg_gpu as u128 * n as u128;
        let loaded_gpu_estimate = avg_gpu as u128 * ls_count as u128;
        let effective_budget = budget.effective_budget();

        if human {
            println!("  ┌─ Memory Budget ──────────────────────────────────────────┐");
            if let Some(mb) = args.budget_mb {
                println!(
                    "  │  Mode:           manual ({} MB)                        │",
                    mb
                );
            } else if budget.total_vram.is_some() {
                println!("  │  Mode:           auto (RAM budget, VRAM via spawn window)│");
            } else {
                println!("  │  Mode:           auto (RAM only, no VRAM detected)      │");
            }
            if budget.total_vram.is_some() {
                println!(
                    "  │  RAM budget:      {:>9}  ({:.0}% of {:.1} GB)       │",
                    human_bytes(budget.ram_budget),
                    ram_fraction * 100.0,
                    budget.total_ram as f64 / 1_073_741_824.0,
                );
                println!(
                    "  │  VRAM budget:     {:>9}  ({:.0}% of {:.1} GB)       │",
                    human_bytes(budget.vram_budget),
                    gpu_fraction * 100.0,
                    budget.total_vram.unwrap_or(0) as f64 / 1_073_741_824.0,
                );
            }
            println!(
                "  │  Effective budget: {:>9}                            │",
                human_bytes(budget.effective_budget())
            );
            println!(
                "  │  Per-frame (avg):  {:>9}                            │",
                human_bytes(budget.frame_size_estimate)
            );
            println!(
                "  │  Max frames:       {:>6}                               │",
                budget.max_frames
            );
            println!("  └──────────────────────────────────────────────────────────┘");
            println!();
            println!("  ┌─ Load Section ────────────────────────────────────────────┐");
            println!(
                "  │  Range:    [{}..{})  ({} frames)                    ",
                ls_start, ls_end, ls_count
            );
            println!(
                "  │  All frames GPU:   {:>9}  ({} frames)           ",
                human_bytes(total_gpu_estimate as u64),
                n
            );
            println!(
                "  │  Loaded GPU:       {:>9}  ({} frames)           ",
                human_bytes(loaded_gpu_estimate as u64),
                ls_count
            );
            let pct = if effective_budget > 0 {
                loaded_gpu_estimate as f64 / effective_budget as f64 * 100.0
            } else {
                0.0
            };
            println!(
                "  │  Budget usage:     {:.1}%                               ",
                pct
            );
            if ls_count < n {
                println!(
                    "  │  ⚠ Only {:.1}% of frames fit in budget               ",
                    ls_count as f64 / n as f64 * 100.0
                );
            } else {
                println!("  │  ✓ All frames fit in budget                            ");
            }
            println!("  └──────────────────────────────────────────────────────────┘");
            println!();

            // ── Timing summary ────────────────────────────────────────────
            println!(
                "  Parse time: {:.1} ms total ({} frames), {:.1} ms/frame avg",
                total_parse_ms,
                sampled.len(),
                total_parse_ms / sampled.len() as f64,
            );

            // ── Projection: full load estimate ────────────────────────────
            let est_full_parse_ms = avg_parse_us as f64 / 1000.0 * n as f64;
            println!(
                "  Est. full parse ({} frames): {:.1} s",
                n,
                est_full_parse_ms / 1000.0,
            );
            println!();
        }

        if args.json_output {
            print_json(
                seq,
                format,
                &sampled,
                &budget,
                &mem_settings,
                ls_start,
                ls_end,
                local_toml_str.is_some(),
                has_global_toml,
                total_parse_ms,
            );
        }
    }

    if human {
        println!("Done.");
    }
}
