//! Timeline debug tool — terminal visualisation of frame timeline and load windows.
//!
//! Shows total frames, budget constraints, and simulates multi-window layouts
//! with ASCII art timeline rendering.
//!
//! Usage:
//!     cargo run --bin timeline_debug --release -- <directory> [OPTIONS]
//!
//! Options:
//!     --windows N      Number of load windows to distribute (default: 1)
//!     --budget-mb N    Manual memory budget override in MB
//!     --width N        Terminal width for visualisation (default: 100)
//!     --playhead F     Show playhead at frame F
//!     --sample N       Frames to sample for size estimation (default: 5)
//!     --at RANGES      Explicit window ranges: "0-100,500-600,1200-1300"

use std::path::{Path, PathBuf};
use std::time::Instant;

use nvml_wrapper::Nvml;
use regex::Regex;
use sysinfo::System;

use seaview::{distribute_sections, LoadSection};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

struct Args {
    directory: PathBuf,
    num_windows: usize,
    budget_mb: Option<u64>,
    term_width: usize,
    playhead: Option<usize>,
    sample_count: usize,
    explicit_ranges: Option<Vec<(usize, usize)>>,
}

fn parse_ranges(s: &str) -> Vec<(usize, usize)> {
    s.split(',')
        .filter_map(|chunk| {
            let parts: Vec<&str> = chunk.trim().split('-').collect();
            if parts.len() == 2 {
                let a = parts[0].trim().parse::<usize>().ok()?;
                let b = parts[1].trim().parse::<usize>().ok()?;
                Some((a, b))
            } else {
                None
            }
        })
        .collect()
}

fn parse_args() -> Args {
    let mut args_iter = std::env::args().skip(1);
    let mut directory = None;
    let mut num_windows = 1;
    let mut budget_mb = None;
    let mut term_width = 100;
    let mut playhead = None;
    let mut sample_count = 5;
    let mut explicit_ranges = None;

    while let Some(arg) = args_iter.next() {
        match arg.as_str() {
            "--windows" | "-w" => {
                num_windows = args_iter
                    .next()
                    .expect("--windows requires a number")
                    .parse()
                    .expect("--windows must be a positive integer");
            }
            "--budget-mb" => {
                budget_mb = Some(
                    args_iter
                        .next()
                        .expect("--budget-mb requires a number")
                        .parse()
                        .expect("--budget-mb must be a positive integer"),
                );
            }
            "--width" => {
                term_width = args_iter
                    .next()
                    .expect("--width requires a number")
                    .parse()
                    .expect("--width must be a positive integer");
            }
            "--playhead" | "-p" => {
                playhead = Some(
                    args_iter
                        .next()
                        .expect("--playhead requires a frame number")
                        .parse()
                        .expect("--playhead must be a non-negative integer"),
                );
            }
            "--sample" => {
                sample_count = args_iter
                    .next()
                    .expect("--sample requires a number")
                    .parse()
                    .expect("--sample must be a positive integer");
            }
            "--at" => {
                let s = args_iter
                    .next()
                    .expect("--at requires ranges like '0-100,500-600'");
                explicit_ranges = Some(parse_ranges(&s));
            }
            "--help" | "-h" => {
                eprintln!("Usage: timeline_debug <directory> [OPTIONS]");
                eprintln!();
                eprintln!("Options:");
                eprintln!("  --windows N, -w N   Number of load windows (default: 1)");
                eprintln!("  --budget-mb N       Manual memory budget in MB");
                eprintln!("  --width N           Terminal width for viz (default: 100)");
                eprintln!("  --playhead F, -p F  Show playhead at frame F");
                eprintln!("  --sample N          Frames to sample for size estimate (default: 5)");
                eprintln!("  --at RANGES         Explicit ranges: '0-100,500-600,1200-1300'");
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
        num_windows,
        budget_mb,
        term_width,
        playhead,
        sample_count,
        explicit_ranges,
    }
}

// ---------------------------------------------------------------------------
// Sequence discovery (lightweight, no Bevy)
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
    let patterns: Vec<(Regex, usize)> = vec![
        (Regex::new(r"^(.+?)[\._-]?(\d{3,6})\.stl$").unwrap(), 2),
        (Regex::new(r"^(.+?)[\._-]?[Tt](\d+)\.stl$").unwrap(), 2),
        (Regex::new(r"^(.+?)(\d{3,})\.stl$").unwrap(), 2),
        (Regex::new(r"^(.+?)[\._-]?(\d{3,6})\.gltf$").unwrap(), 2),
        (Regex::new(r"^(.+?)(\d{3,})\.gltf$").unwrap(), 2),
        (Regex::new(r"^(.+?)[\._-]?(\d{3,6})\.glb$").unwrap(), 2),
        (Regex::new(r"^(.+?)(\d{3,})\.glb$").unwrap(), 2),
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

        for (regex, frame_group) in &patterns {
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
// Mesh parsing (headless)
// ---------------------------------------------------------------------------

fn estimate_gpu_bytes(vertex_count: usize, index_count: usize) -> u64 {
    const BYTES_PER_VERTEX: usize = 12 + 12 + 8; // pos + normal + uv = 32
    const BYTES_PER_INDEX: usize = 4;
    const OVERHEAD: f64 = 1.1;
    let vertex_bytes = vertex_count * BYTES_PER_VERTEX;
    let index_bytes = index_count * BYTES_PER_INDEX;
    ((vertex_bytes + index_bytes) as f64 * OVERHEAD) as u64
}

fn parse_glb_frame_size(path: &Path) -> Result<u64, String> {
    let (document, _buffers, _) = gltf::import(path).map_err(|e| format!("gltf import: {}", e))?;

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

    Ok(estimate_gpu_bytes(total_vertices, total_indices))
}

fn parse_stl_frame_size(path: &Path) -> Result<u64, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("open: {}", e))?;
    let mut reader = std::io::BufReader::new(&mut file);
    let stl = stl_io::read_stl(&mut reader).map_err(|e| format!("STL parse: {}", e))?;
    let tri = stl.faces.len();
    Ok(estimate_gpu_bytes(tri * 3, tri * 3))
}

fn sample_frame_size(seq: &DiscoveredSequence, sample_count: usize) -> u64 {
    let n = seq.frames.len();
    let count = sample_count.min(n).max(1);

    let indices: Vec<usize> = if count >= n {
        (0..n).collect()
    } else {
        (0..count)
            .map(|i| i * (n - 1) / (count - 1).max(1))
            .collect()
    };

    let mut sizes: Vec<u64> = Vec::new();
    for &idx in &indices {
        let path = &seq.frames[idx].path;
        let result = match seq.format {
            MeshFormat::Glb | MeshFormat::Gltf => parse_glb_frame_size(path),
            MeshFormat::Stl => parse_stl_frame_size(path),
        };
        if let Ok(sz) = result {
            sizes.push(sz);
        }
    }

    if sizes.is_empty() {
        0
    } else {
        sizes.iter().sum::<u64>() / sizes.len() as u64
    }
}

// ---------------------------------------------------------------------------
// Hardware detection
// ---------------------------------------------------------------------------

struct HwInfo {
    ram_total: u64,
    ram_available: u64,
    gpu_name: Option<String>,
    vram_total: Option<u64>,
    vram_free: Option<u64>,
}

fn detect_hardware() -> HwInfo {
    let mut sys = System::new_all();
    sys.refresh_memory();

    let mut gpu_name = None;
    let mut vram_total = None;
    let mut vram_free = None;

    if let Ok(nvml) = Nvml::init() {
        if let Ok(device) = nvml.device_by_index(0) {
            gpu_name = device.name().ok();
            if let Ok(mem) = device.memory_info() {
                vram_total = Some(mem.total);
                vram_free = Some(mem.free);
            }
        }
    }

    HwInfo {
        ram_total: sys.total_memory(),
        ram_available: sys.available_memory(),
        gpu_name,
        vram_total,
        vram_free,
    }
}

// ---------------------------------------------------------------------------
// Load windows
// ---------------------------------------------------------------------------

/// Parse explicit ranges and clamp to total_frames
fn parse_explicit_windows(ranges: &[(usize, usize)], total_frames: usize) -> Vec<LoadSection> {
    ranges
        .iter()
        .map(|&(s, e)| LoadSection::new(s.min(total_frames), e.min(total_frames)))
        .filter(|w| w.frame_count() > 0)
        .collect()
}

// ---------------------------------------------------------------------------
// Settings loading
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
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg).join("seaview").join("seaview.toml");
        if path.exists() {
            return std::fs::read_to_string(&path).ok();
        }
    }
    if let Ok(home) = std::env::var("HOME") {
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

fn extract_fractions(global_toml: Option<&str>) -> (f32, f32) {
    global_toml
        .and_then(|s| {
            let table: toml::Table = toml::from_str(s).ok()?;
            let mem = table.get("memory")?.as_table()?;
            let rf = mem
                .get("ram_fraction")
                .and_then(|v| v.as_float())
                .unwrap_or(0.5) as f32;
            let gf = mem
                .get("gpu_fraction")
                .and_then(|v| v.as_float())
                .unwrap_or(0.5) as f32;
            Some((rf, gf))
        })
        .unwrap_or((0.5, 0.5))
}

// ---------------------------------------------------------------------------
// Terminal rendering
// ---------------------------------------------------------------------------

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

/// Render a single-line ASCII timeline bar.
///
/// `width` is the number of character cells for the bar content (excluding brackets).
///
/// Characters:
///   `─`  unloaded frame
///   `█`  loaded frame (in a window)
///   `▓`  loaded + playhead on this segment
///   `▏`  playhead on unloaded segment
fn render_timeline(
    total_frames: usize,
    windows: &[LoadSection],
    playhead: Option<usize>,
    width: usize,
) -> Vec<String> {
    if total_frames == 0 || width == 0 {
        return vec!["(empty sequence)".to_string()];
    }

    let bar_width = width.max(20);

    // Build a character array for the bar
    let mut bar: Vec<char> = vec!['─'; bar_width];

    // Fill loaded regions
    for w in windows {
        let col_start = (w.start as f64 / total_frames as f64 * bar_width as f64) as usize;
        let col_end = (w.end as f64 / total_frames as f64 * bar_width as f64).ceil() as usize;
        for col in col_start..col_end.min(bar_width) {
            bar[col] = '█';
        }
    }

    // Place playhead
    if let Some(ph) = playhead {
        if ph < total_frames {
            let col = (ph as f64 / total_frames as f64 * bar_width as f64) as usize;
            let col = col.min(bar_width - 1);
            let in_window = windows.iter().any(|w| w.contains(ph));
            bar[col] = if in_window { '▓' } else { '▏' };
        }
    }

    // Frame number labels
    let label_line = format!(
        "{:<width$}",
        format!("0{:>pad$}", total_frames, pad = bar_width - 1),
        width = bar_width,
    );

    // Compose output lines
    let mut lines = Vec::new();

    // Top: frame scale
    lines.push(format!(" {}", label_line));

    // Bar with brackets
    let bar_str: String = bar.iter().collect();
    lines.push(format!("[{}]", bar_str));

    // Window annotations below the bar
    if !windows.is_empty() {
        let mut anno: Vec<char> = vec![' '; bar_width + 2]; // +2 for brackets

        for (i, w) in windows.iter().enumerate() {
            let col_start = 1 + (w.start as f64 / total_frames as f64 * bar_width as f64) as usize;
            let label = format!("W{}: {}-{} ({})", i, w.start, w.end, w.frame_count());

            // Place label starting at col_start, don't overflow
            let start_col = col_start.min(anno.len().saturating_sub(label.len()));
            for (j, ch) in label.chars().enumerate() {
                let pos = start_col + j;
                if pos < anno.len() {
                    anno[pos] = ch;
                }
            }
        }

        lines.push(anno.iter().collect());
    }

    lines
}

/// Render a detailed multi-line view with per-window breakdown.
fn render_detail_view(
    total_frames: usize,
    windows: &[LoadSection],
    playhead: Option<usize>,
    frame_bytes: u64,
    budget_bytes: u64,
    bar_width: usize,
) -> Vec<String> {
    let mut lines = Vec::new();

    let total_loaded: usize = windows.iter().map(|w| w.frame_count()).sum();
    let loaded_bytes = frame_bytes as u128 * total_loaded as u128;
    let total_bytes = frame_bytes as u128 * total_frames as u128;

    lines.push(String::new());
    lines.push(format!("  Total frames:  {}", total_frames));
    lines.push(format!(
        "  Loaded frames: {} across {} window(s)",
        total_loaded,
        windows.len()
    ));
    lines.push(format!(
        "  Coverage:      {:.1}%",
        if total_frames > 0 {
            total_loaded as f64 / total_frames as f64 * 100.0
        } else {
            0.0
        }
    ));
    if let Some(ph) = playhead {
        let in_window = windows.iter().any(|w| w.contains(ph));
        lines.push(format!(
            "  Playhead:      frame {} {}",
            ph,
            if in_window {
                "✓ in loaded window"
            } else {
                "✗ NOT in any window"
            }
        ));
    }
    lines.push(String::new());
    lines.push(format!(
        "  Memory: {} loaded / {} total sequence / {} budget",
        human_bytes(loaded_bytes as u64),
        human_bytes(total_bytes as u64),
        human_bytes(budget_bytes),
    ));
    if budget_bytes > 0 {
        lines.push(format!(
            "  Budget usage: {:.1}%",
            loaded_bytes as f64 / budget_bytes as f64 * 100.0
        ));
    }

    lines.push(String::new());

    // Per-window table
    lines.push("  ┌─────┬──────────────────┬────────┬──────────┬────────────┐".to_string());
    lines.push("  │  #  │  Range           │ Frames │ GPU mem  │ % of total │".to_string());
    lines.push("  ├─────┼──────────────────┼────────┼──────────┼────────────┤".to_string());

    for (i, w) in windows.iter().enumerate() {
        let mem = frame_bytes as u128 * w.frame_count() as u128;
        let pct = if total_frames > 0 {
            w.frame_count() as f64 / total_frames as f64 * 100.0
        } else {
            0.0
        };
        lines.push(format!(
            "  │ W{:<2} │ {:>6} - {:<6}  │ {:>6} │ {:>8} │ {:>8.1}%  │",
            i,
            w.start,
            w.end,
            w.frame_count(),
            human_bytes(mem as u64),
            pct,
        ));
    }
    lines.push("  └─────┴──────────────────┴────────┴──────────┴────────────┘".to_string());

    // Gaps between windows
    if windows.len() > 1 {
        lines.push(String::new());
        lines.push("  Gaps:".to_string());
        // Gap before first window
        if windows[0].start > 0 {
            lines.push(format!(
                "    [0..{})  {} frames unloaded",
                windows[0].start, windows[0].start,
            ));
        }
        for i in 0..windows.len() - 1 {
            let gap_start = windows[i].end;
            let gap_end = windows[i + 1].start;
            if gap_end > gap_start {
                lines.push(format!(
                    "    [{}..{})  {} frames unloaded",
                    gap_start,
                    gap_end,
                    gap_end - gap_start,
                ));
            }
        }
        // Gap after last window
        if let Some(last) = windows.last() {
            if last.end < total_frames {
                lines.push(format!(
                    "    [{}..{})  {} frames unloaded",
                    last.end,
                    total_frames,
                    total_frames - last.end,
                ));
            }
        }
    }

    // Timeline visualisation
    lines.push(String::new());
    lines.push("  Timeline:".to_string());
    for line in render_timeline(total_frames, windows, playhead, bar_width) {
        lines.push(format!("  {}", line));
    }

    // Zoomed view of each window
    if windows.len() > 1 || (windows.len() == 1 && total_frames > windows[0].frame_count() * 3) {
        lines.push(String::new());
        lines.push("  Window detail:".to_string());
        for (i, w) in windows.iter().enumerate() {
            // Show context: 10% padding on each side
            let pad = (w.frame_count() as f64 * 0.3).max(10.0) as usize;
            let ctx_start = w.start.saturating_sub(pad);
            let ctx_end = (w.end + pad).min(total_frames);
            let ctx_total = ctx_end - ctx_start;

            // Remap this window into the context range
            let local_windows = vec![LoadSection::new(w.start - ctx_start, w.end - ctx_start)];
            let local_playhead = playhead
                .filter(|&ph| ph >= ctx_start && ph < ctx_end)
                .map(|ph| ph - ctx_start);

            let zoom_width = bar_width.min(60);
            let zoom_lines = render_timeline(ctx_total, &local_windows, local_playhead, zoom_width);
            lines.push(format!(
                "    W{}: frames {}..{} (context {}..{})",
                i, w.start, w.end, ctx_start, ctx_end
            ));
            for zl in &zoom_lines {
                lines.push(format!("    {}", zl));
            }
        }
    }

    lines
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

    // ── Settings ──────────────────────────────────────────────────────────
    let _local_toml_str = load_local_settings(dir);
    let global_toml_str = load_global_settings();
    let (ram_fraction, gpu_fraction) = extract_fractions(global_toml_str.as_deref());

    // ── Hardware ──────────────────────────────────────────────────────────
    let hw = detect_hardware();

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║                  Seaview Timeline Debug                         ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    println!(
        "  RAM:  {:.1} GB total, {:.1} GB available (budget {:.0}% = {:.1} GB)",
        hw.ram_total as f64 / 1_073_741_824.0,
        hw.ram_available as f64 / 1_073_741_824.0,
        ram_fraction * 100.0,
        hw.ram_total as f64 * ram_fraction as f64 / 1_073_741_824.0,
    );
    if let (Some(name), Some(vt), Some(vf)) = (&hw.gpu_name, hw.vram_total, hw.vram_free) {
        println!(
            "  GPU:  {} — {:.1} GB total, {:.1} GB free (budget {:.0}% = {:.1} GB)",
            name,
            vt as f64 / 1_073_741_824.0,
            vf as f64 / 1_073_741_824.0,
            gpu_fraction * 100.0,
            vt as f64 * gpu_fraction as f64 / 1_073_741_824.0,
        );
    } else {
        println!("  GPU:  NVML unavailable — no VRAM detection");
    }
    println!();

    // ── Effective budget ──────────────────────────────────────────────────
    let ram_budget = (hw.ram_total as f64 * ram_fraction as f64) as u64;
    let vram_budget = hw
        .vram_total
        .map(|v| (v as f64 * gpu_fraction as f64) as u64);

    let effective_budget = if let Some(mb) = args.budget_mb {
        let b = mb * 1024 * 1024;
        println!("  Budget: {} (manual override)", human_bytes(b));
        b
    } else {
        let b = match vram_budget {
            Some(vb) => ram_budget.min(vb),
            None => ram_budget,
        };
        println!(
            "  Budget: {} (effective = min of RAM, VRAM)",
            human_bytes(b)
        );
        b
    };
    println!();

    // ── Discovery ─────────────────────────────────────────────────────────
    let t0 = Instant::now();
    let sequences = discover_sequences(dir);
    let discover_ms = t0.elapsed().as_secs_f64() * 1000.0;

    if sequences.is_empty() {
        println!("  No sequences found in {:?}", dir);
        std::process::exit(0);
    }

    println!(
        "  Found {} sequence(s) in {:.1} ms",
        sequences.len(),
        discover_ms
    );
    println!();

    // ── Per-sequence ──────────────────────────────────────────────────────
    for seq in &sequences {
        let n = seq.frames.len();
        let total_disk: u64 = seq.frames.iter().map(|f| f.file_size).sum();

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!(
            "  Sequence: \"{}\"  |  {} frames  |  {}  |  {} on disk",
            seq.name,
            n,
            seq.format.label(),
            human_bytes(total_disk),
        );
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // Sample frame sizes
        print!(
            "  Sampling {} frames for size estimate... ",
            args.sample_count
        );
        let avg_frame_bytes = sample_frame_size(seq, args.sample_count);
        if avg_frame_bytes == 0 {
            println!("FAILED (no frames parsed)");
            continue;
        }
        println!("{}/frame", human_bytes(avg_frame_bytes));

        let max_frames = if avg_frame_bytes > 0 {
            (effective_budget / avg_frame_bytes) as usize
        } else {
            n
        };

        println!(
            "  Budget fits {} frames (of {} total = {:.1}%)",
            max_frames,
            n,
            max_frames as f64 / n as f64 * 100.0
        );

        // ── Build windows ─────────────────────────────────────────────────
        let windows = if let Some(ref ranges) = args.explicit_ranges {
            let ws = parse_explicit_windows(ranges, n);
            let total_loaded: usize = ws.iter().map(|w| w.frame_count()).sum();
            if total_loaded > max_frames {
                println!(
                    "  ⚠ Explicit ranges request {} frames but budget allows {} — showing anyway",
                    total_loaded, max_frames
                );
            }
            ws
        } else {
            distribute_sections(n, max_frames, args.num_windows)
        };

        let bar_width = args.term_width.saturating_sub(6).max(20);

        // ── Render ────────────────────────────────────────────────────────
        for line in render_detail_view(
            n,
            &windows,
            args.playhead,
            avg_frame_bytes,
            effective_budget,
            bar_width,
        ) {
            println!("{}", line);
        }

        // ── Compare layouts ───────────────────────────────────────────────
        if args.explicit_ranges.is_none() && args.num_windows == 1 {
            println!();
            println!("  ── Alternative layouts ──");
            for num_w in [2, 3, 4, 5] {
                if max_frames < num_w * 2 {
                    break;
                }
                let alt_windows = distribute_sections(n, max_frames, num_w);
                let alt_total: usize = alt_windows.iter().map(|w| w.frame_count()).sum();
                println!();
                println!(
                    "  {} windows ({} frames loaded, {:.1}% coverage):",
                    num_w,
                    alt_total,
                    alt_total as f64 / n as f64 * 100.0,
                );
                for line in render_timeline(n, &alt_windows, args.playhead, bar_width) {
                    println!("  {}", line);
                }
                // Compact range summary
                let ranges_str: Vec<String> = alt_windows
                    .iter()
                    .enumerate()
                    .map(|(i, w)| format!("W{}:[{}-{})", i, w.start, w.end))
                    .collect();
                println!("  {}", ranges_str.join("  "));
            }
        }

        println!();
    }

    println!("Done.");
}
