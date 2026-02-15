//! Headless debug loader for sequence analysis without GPU/window.
//!
//! Discovers sequences, parses mesh files directly (GLB/glTF/STL), computes
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
use regex::Regex;
use sysinfo::System;

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
// Lightweight sequence discovery (no Bevy dependency)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshFormat {
    Stl,
    Gltf,
    Glb,
}

impl MeshFormat {
    fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "stl" => Some(Self::Stl),
            "gltf" => Some(Self::Gltf),
            "glb" => Some(Self::Glb),
            _ => None,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Stl => "STL",
            Self::Gltf => "glTF",
            Self::Glb => "GLB",
        }
    }
}

#[derive(Debug, Clone)]
struct FrameFile {
    path: PathBuf,
    frame_number: usize,
    file_size: u64,
}

#[derive(Debug)]
struct DiscoveredSequence {
    name: String,
    format: MeshFormat,
    frames: Vec<FrameFile>,
}

fn discover_sequences(directory: &Path) -> Vec<DiscoveredSequence> {
    let patterns: Vec<(&str, Regex, usize)> = vec![
        // STL
        (
            "stl",
            Regex::new(r"^(.+?)[\._-]?(\d{3,6})\.stl$").unwrap(),
            2,
        ),
        (
            "stl",
            Regex::new(r"^(.+?)[\._-]?[Tt](\d+)\.stl$").unwrap(),
            2,
        ),
        ("stl", Regex::new(r"^(.+?)(\d{3,})\.stl$").unwrap(), 2),
        // glTF
        (
            "gltf",
            Regex::new(r"^(.+?)[\._-]?(\d{3,6})\.gltf$").unwrap(),
            2,
        ),
        ("gltf", Regex::new(r"^(.+?)(\d{3,})\.gltf$").unwrap(), 2),
        // GLB
        (
            "glb",
            Regex::new(r"^(.+?)[\._-]?(\d{3,6})\.glb$").unwrap(),
            2,
        ),
        ("glb", Regex::new(r"^(.+?)(\d{3,})\.glb$").unwrap(), 2),
    ];

    let mut groups: std::collections::HashMap<String, Vec<FrameFile>> =
        std::collections::HashMap::new();

    let entries = match std::fs::read_dir(directory) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Cannot read directory {:?}: {}", directory, e);
            return Vec::new();
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };

        for (_fmt_name, regex, frame_group) in &patterns {
            if let Some(captures) = regex.captures(filename) {
                if let (Some(base_match), Some(frame_match)) =
                    (captures.get(1), captures.get(*frame_group))
                {
                    if let Ok(frame_number) = frame_match.as_str().parse::<usize>() {
                        let key = base_match.as_str().to_string();
                        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        groups.entry(key).or_default().push(FrameFile {
                            path: path.clone(),
                            frame_number,
                            file_size,
                        });
                        break;
                    }
                }
            }
        }
    }

    let mut sequences = Vec::new();
    for (name, mut frames) in groups {
        if frames.len() < 2 {
            continue;
        }
        frames.sort_by_key(|f| f.frame_number);

        let format = frames
            .first()
            .and_then(|f| f.path.extension())
            .and_then(|e| e.to_str())
            .and_then(MeshFormat::from_extension)
            .unwrap_or(MeshFormat::Glb);

        sequences.push(DiscoveredSequence {
            name,
            format,
            frames,
        });
    }
    sequences.sort_by(|a, b| a.name.cmp(&b.name));
    sequences
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

/// Estimate GPU memory from vertex + index counts (matches memory_budget.rs logic)
fn estimate_gpu_bytes(vertex_count: usize, index_count: usize) -> u64 {
    const BYTES_PER_VERTEX: usize = 12 + 12 + 8; // pos + normal + uv = 32
    const BYTES_PER_INDEX: usize = 4;
    const OVERHEAD: f64 = 1.1;

    let vertex_bytes = vertex_count * BYTES_PER_VERTEX;
    let index_bytes = index_count * BYTES_PER_INDEX;
    ((vertex_bytes + index_bytes) as f64 * OVERHEAD) as u64
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
        estimated_gpu_bytes: estimate_gpu_bytes(total_vertices, total_indices),
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
        estimated_gpu_bytes: estimate_gpu_bytes(vertex_count, index_count),
        parse_us,
    })
}

fn parse_frame(path: &Path, format: MeshFormat) -> Result<MeshStats, String> {
    match format {
        MeshFormat::Glb | MeshFormat::Gltf => parse_glb_stats(path),
        MeshFormat::Stl => parse_stl_stats(path),
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

fn load_global_settings() -> Option<String> {
    // XDG_CONFIG_HOME
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg).join("seaview").join("seaview.toml");
        if path.exists() {
            return std::fs::read_to_string(&path).ok();
        }
    }
    // ~/.config/seaview/seaview.toml
    if let Some(home) = std::env::var("HOME").ok() {
        let path = PathBuf::from(home)
            .join(".config")
            .join("seaview")
            .join("seaview.toml");
        if path.exists() {
            return std::fs::read_to_string(&path).ok();
        }
    }
    None
}

// ---------------------------------------------------------------------------
// RAM detection
// ---------------------------------------------------------------------------

struct RamInfo {
    total_bytes: u64,
    available_bytes: u64,
}

fn detect_ram() -> RamInfo {
    let mut sys = System::new_all();
    sys.refresh_memory();
    RamInfo {
        total_bytes: sys.total_memory(),
        available_bytes: sys.available_memory(),
    }
}

// ---------------------------------------------------------------------------
// VRAM detection (NVML)
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
// Budget computation
// ---------------------------------------------------------------------------

struct BudgetReport {
    ram_total_gb: f64,
    ram_available_gb: f64,
    ram_budget_gb: f64,
    ram_fraction: f32,
    vram_total_bytes: Option<u64>,
    vram_budget_bytes: Option<u64>,
    gpu_fraction: f32,
    /// Effective budget: min(RAM budget, VRAM budget) or RAM-only if no GPU
    effective_budget_bytes: u64,
    /// Per-frame estimate
    frame_bytes: u64,
    /// Max frames that fit
    max_frames: usize,
}

fn compute_budget(
    ram: &RamInfo,
    vram_total: Option<u64>,
    avg_frame_bytes: u64,
    manual_budget_mb: Option<u64>,
    ram_fraction: f32,
    gpu_fraction: f32,
) -> BudgetReport {
    let gb = 1_073_741_824.0_f64;

    let ram_budget_bytes = (ram.total_bytes as f64 * ram_fraction as f64) as u64;
    let vram_budget_bytes = vram_total.map(|v| (v as f64 * gpu_fraction as f64) as u64);

    let effective_budget_bytes = if let Some(mb) = manual_budget_mb {
        mb * 1024 * 1024
    } else {
        match vram_budget_bytes {
            Some(vb) => ram_budget_bytes.min(vb),
            None => ram_budget_bytes,
        }
    };

    let max_frames = if avg_frame_bytes > 0 {
        (effective_budget_bytes / avg_frame_bytes) as usize
    } else {
        usize::MAX
    };

    BudgetReport {
        ram_total_gb: ram.total_bytes as f64 / gb,
        ram_available_gb: ram.available_bytes as f64 / gb,
        ram_budget_gb: ram_budget_bytes as f64 / gb,
        ram_fraction,
        vram_total_bytes: vram_total,
        vram_budget_bytes,
        gpu_fraction,
        effective_budget_bytes,
        frame_bytes: avg_frame_bytes,
        max_frames,
    }
}

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.2} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

// ---------------------------------------------------------------------------
// JSON output
// ---------------------------------------------------------------------------

fn print_json(
    seq: &DiscoveredSequence,
    sampled: &[MeshStats],
    budget: &BudgetReport,
    load_section_start: usize,
    load_section_end: usize,
    local_toml: bool,
    global_toml: bool,
    total_parse_ms: f64,
) {
    // Minimal JSON without serde_json — keep deps light
    println!("{{");
    println!("  \"sequence\": {{");
    println!("    \"name\": {:?},", seq.name);
    println!("    \"format\": {:?},", seq.format.label());
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
    println!("    \"ram_total_gb\": {:.2},", budget.ram_total_gb);
    println!("    \"ram_available_gb\": {:.2},", budget.ram_available_gb);
    println!("    \"ram_budget_gb\": {:.2},", budget.ram_budget_gb);
    println!("    \"ram_fraction\": {},", budget.ram_fraction);
    if let Some(vt) = budget.vram_total_bytes {
        println!("    \"vram_total_bytes\": {},", vt);
    }
    if let Some(vb) = budget.vram_budget_bytes {
        println!("    \"vram_budget_bytes\": {},", vb);
    }
    println!("    \"gpu_fraction\": {},", budget.gpu_fraction);
    println!(
        "    \"effective_budget_bytes\": {},",
        budget.effective_budget_bytes
    );
    println!("    \"frame_bytes\": {},", budget.frame_bytes);
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
    let global_toml_str = load_global_settings();

    // Extract memory fractions from global config (best effort)
    let (ram_fraction, gpu_fraction): (f32, f32) = global_toml_str
        .as_deref()
        .and_then(|s| {
            let table: toml::Table = toml::from_str(s).ok()?;
            let mem = table.get("memory")?.as_table()?;
            let rf = mem
                .get("ram_fraction")
                .and_then(|v| v.as_float())
                .map(|f| f as f32)
                .unwrap_or(0.5);
            let gf = mem
                .get("gpu_fraction")
                .and_then(|v| v.as_float())
                .map(|f| f as f32)
                .unwrap_or(0.5);
            Some((rf, gf))
        })
        .unwrap_or((0.5, 0.5));

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
            if global_toml_str.is_some() {
                "found ✓"
            } else {
                "none"
            }
        );
        if cfg_ls_start != 0 || cfg_ls_end != usize::MAX {
            println!("Config load_section: [{}..{})", cfg_ls_start, cfg_ls_end);
        }
        println!();
    }

    // ── Discovery ─────────────────────────────────────────────────────────
    let t_discover = Instant::now();
    let sequences = discover_sequences(dir);
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

    // ── RAM ───────────────────────────────────────────────────────────────
    let ram = detect_ram();

    if human {
        println!(
            "RAM: {:.1} GB total, {:.1} GB available",
            ram.total_bytes as f64 / 1_073_741_824.0,
            ram.available_bytes as f64 / 1_073_741_824.0,
        );
        println!(
            "     fraction={:.0}% → budget {:.1} GB",
            ram_fraction * 100.0,
            ram.total_bytes as f64 * ram_fraction as f64 / 1_073_741_824.0,
        );
        println!();
    }

    // ── GPU / VRAM ────────────────────────────────────────────────────────
    let gpus = detect_gpus();
    // Use first GPU for budget (future: user-selectable)
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

    let vram_total = primary_gpu.map(|g| g.total_vram);

    // ── Per-sequence analysis ─────────────────────────────────────────────
    for seq in &sequences {
        let n = seq.frames.len();
        let total_file_bytes: u64 = seq.frames.iter().map(|f| f.file_size).sum();

        if human {
            println!("━━━ Sequence: \"{}\" ━━━", seq.name);
            println!("  Format:       {}", seq.format.label());
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
            match parse_frame(&frame.path, seq.format) {
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

        // ── Budget ────────────────────────────────────────────────────────
        let budget = compute_budget(
            &ram,
            vram_total,
            avg_gpu,
            args.budget_mb,
            ram_fraction,
            gpu_fraction,
        );

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

        if human {
            println!("  ┌─ Memory Budget ──────────────────────────────────────────┐");
            if let Some(mb) = args.budget_mb {
                println!(
                    "  │  Mode:           manual ({} MB)                        │",
                    mb
                );
            } else if budget.vram_budget_bytes.is_some() {
                println!("  │  Mode:           auto (min of RAM, VRAM)                │");
            } else {
                println!("  │  Mode:           auto (RAM only, no VRAM detected)      │");
            }
            if let Some(vb) = budget.vram_budget_bytes {
                println!(
                    "  │  RAM budget:      {:>9}  ({:.0}% of {:.1} GB)       │",
                    human_bytes((ram.total_bytes as f64 * ram_fraction as f64) as u64),
                    ram_fraction * 100.0,
                    budget.ram_total_gb,
                );
                println!(
                    "  │  VRAM budget:     {:>9}  ({:.0}% of {:.1} GB)       │",
                    human_bytes(vb),
                    budget.gpu_fraction * 100.0,
                    budget.vram_total_bytes.unwrap_or(0) as f64 / 1_073_741_824.0,
                );
            }
            println!(
                "  │  Effective budget: {:>9}                            │",
                human_bytes(budget.effective_budget_bytes)
            );
            println!(
                "  │  Per-frame (avg):  {:>9}                            │",
                human_bytes(budget.frame_bytes)
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
            let pct = if budget.effective_budget_bytes > 0 {
                loaded_gpu_estimate as f64 / budget.effective_budget_bytes as f64 * 100.0
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
                &sampled,
                &budget,
                ls_start,
                ls_end,
                local_toml_str.is_some(),
                global_toml_str.is_some(),
                total_parse_ms,
            );
        }
    }

    if human {
        println!("Done.");
    }
}
