use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

use self::processor::{FileProcessor, Processor};

mod module_path;
mod processor;
mod transformer;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Input file or directory path
    input_path: PathBuf,

    /// Output directory path (default: "code-context")
    #[arg(short = 'o', long = "output-dir")]
    output_dir_name: Option<String>,

    /// Remove all comments (including doc comments)
    #[arg(long)]
    no_comments: bool,

    /// Print processing statistics
    #[arg(long)]
    no_stats: bool,

    /// Run without writing output files
    #[arg(long)]
    dry_run: bool,

    /// Output all files into a single combined file
    #[arg(long)]
    single_file: bool,
}

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    tracing::info!("Starting code context generation...");
    tracing::debug!("Input path: {:?}", cli.input_path);

    let processor = create_processor(&cli);
    let stats = processor
        .process_path(&cli.input_path, cli.output_dir_name.as_deref())
        .with_context(|| format!("Failed to process path: {}", cli.input_path.display()))?;

    if !cli.no_stats {
        println!("\nProcessing Statistics:");
        println!("Files processed: {}", stats.files_processed);
        println!("Total input size: {} bytes", stats.input_size);
        println!("Total output size: {} bytes", stats.output_size);
        println!("Size reduction: {:.1}%", stats.reduction_percentage());
    }

    tracing::info!("Processing complete!");
    Ok(())
}

fn create_processor(cli: &Cli) -> impl Processor {
    FileProcessor::with_options(cli.no_comments, cli.dry_run, cli.single_file)
}
