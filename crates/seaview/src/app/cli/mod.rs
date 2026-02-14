use bevy::prelude::Resource;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Resource)]
#[command(name = "seaview")]
#[command(about = "A 3D mesh viewer for STL files", long_about = None)]
pub struct Args {
    /// Path to an STL file or directory containing a sequence
    pub path: Option<PathBuf>,

    /// Enable verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Source coordinate system: 'yup' (graphics/Bevy default), 'zup' (CAD/GIS), 'fluidx3d' (CFD)
    #[arg(short = 's', long, default_value = "yup", value_name = "SYSTEM")]
    pub source_coordinates: String,

    /// Enable network mesh receiving on the specified port
    #[arg(long, short = 'n')]
    pub network_port: Option<u16>,

    /// Watch directory for new frames and auto-load them
    #[arg(short = 'w', long)]
    pub watch: bool,
}

impl Args {
    pub fn parse_args() -> Self {
        Args::parse()
    }
}
