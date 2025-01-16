use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

use self::processor::{FileProcessor, Processor};

mod module_path;
mod processor;
mod test_utils;
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

    /// Remove function bodies except for string/serialization methods
    #[arg(long)]
    no_function_bodies: bool,

    /// Don't print processing statistics
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
    // Initialize logging, using try_init() to handle errors gracefully
    let _ = tracing_subscriber::fmt::try_init();

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
    FileProcessor::with_options(
        cli.no_comments,
        cli.no_function_bodies,
        cli.dry_run,
        cli.single_file,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_cli_parsing() {
        let args = vec![
            "program",
            "--no-comments",
            "--dry-run",
            "--single-file",
            "--no-stats",
            "-o",
            "output-dir",
            "input-path",
        ];
        let cli = Cli::try_parse_from(args).unwrap();

        assert!(cli.no_comments);
        assert!(cli.dry_run);
        assert!(cli.single_file);
        assert!(cli.no_stats);
        assert_eq!(cli.output_dir_name.unwrap(), "output-dir");
        assert_eq!(cli.input_path, PathBuf::from("input-path"));
    }

    #[test]
    fn test_cli_all_options() -> Result<()> {
        let args = vec![
            "program",
            "input.rs",
            "--no-comments",
            "--no-stats",
            "--dry-run",
            "--single-file",
            "-o",
            "custom-output",
        ];

        let cli = Cli::try_parse_from(args)?;

        assert!(cli.no_comments);
        assert!(cli.no_stats);
        assert!(cli.dry_run);
        assert!(cli.single_file);
        assert_eq!(cli.output_dir_name.as_deref(), Some("custom-output"));
        assert_eq!(cli.input_path.to_str().unwrap(), "input.rs");

        Ok(())
    }

    #[test]
    fn test_processor_creation() {
        let cli = Cli {
            input_path: PathBuf::from("test"),
            output_dir_name: None,
            no_comments: true,
            no_function_bodies: false,
            no_stats: false,
            dry_run: true,
            single_file: true,
        };

        let processor = create_processor(&cli);
        assert!(processor.no_comments());
        assert!(processor.dry_run());
        assert!(processor.single_file());
    }

    #[test]
    fn test_main_with_invalid_path() {
        let args = vec!["program", "nonexistent-path"];
        let cli = Cli::try_parse_from(args).unwrap();

        let result = cli.input_path.try_exists();
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_main_with_valid_path() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "fn main() {}")?;

        let args = vec!["program", test_file.to_str().unwrap(), "--dry-run"];
        let cli = Cli::try_parse_from(args).unwrap();

        let processor = create_processor(&cli);
        let stats = processor.process_path(&cli.input_path, cli.output_dir_name.as_deref())?;

        assert_eq!(stats.files_processed, 1);
        assert!(stats.input_size > 0);
        Ok(())
    }

    #[test]
    fn test_main_with_output_dir() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "fn main() {}")?;

        let output_dir = "test-output";
        let args = vec!["program", test_file.to_str().unwrap(), "-o", output_dir];
        let cli = Cli::try_parse_from(args).unwrap();

        println!("CLI dry_run: {}", cli.dry_run);
        let processor = create_processor(&cli);
        println!("Processor dry_run: {}", processor.dry_run());

        if !cli.dry_run {
            let output_dir = FileProcessor::get_output_path(&cli.input_path, Some(output_dir))?;
            let result = processor.process_path(&cli.input_path, cli.output_dir_name.as_deref());
            println!("Process result: {:?}", result);
            println!("Output dir exists: {}", output_dir.exists());
            result?;
            assert!(
                output_dir.exists(),
                "Output directory was not created at {:?}",
                output_dir
            );
        }
        Ok(())
    }

    #[test]
    fn test_main_with_single_file() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "fn main() {}")?;

        let args = vec!["program", test_file.to_str().unwrap(), "--single-file"];
        let cli = Cli::try_parse_from(args).unwrap();

        let processor = create_processor(&cli);
        let stats = processor.process_path(&cli.input_path, cli.output_dir_name.as_deref())?;

        assert_eq!(stats.files_processed, 1);
        Ok(())
    }

    #[test]
    fn test_main_error_handling() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let nonexistent_file = temp_dir.path().join("does_not_exist.rs");

        let args = vec!["program", nonexistent_file.to_str().unwrap(), "--dry-run"];

        let cli = Cli::try_parse_from(args)?;
        let processor = create_processor(&cli);
        let result = processor.process_path(&cli.input_path, cli.output_dir_name.as_deref());

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));

        Ok(())
    }

    #[test]
    fn test_main_with_stats_output() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let test_file = temp_dir.path().join("test.rs");
        fs::write(
            &test_file,
            r#"
            fn main() {
                println!("Starting program");
                let mut sum = 0;
                for i in 0..100 {
                    sum += i;
                    println!("Current sum: {}", sum);
                }
                println!("Final sum: {}", sum);
            }
        "#,
        )?;

        let args = vec![
            "program",
            test_file.to_str().unwrap(),
            "--no-comments",
            "--no-function-bodies",
        ];
        let cli = Cli::try_parse_from(args)?;
        let stats =
            create_processor(&cli).process_path(&cli.input_path, cli.output_dir_name.as_deref())?;

        assert!(stats.reduction_percentage() > 0.0);
        Ok(())
    }

    #[test]
    fn test_main_with_logging() -> Result<()> {
        // Use try_init() instead of init() to handle case where logger is already initialized
        let _ = tracing_subscriber::fmt::try_init();

        let temp_dir = TempDir::new()?;
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "fn main() {}")?;

        let cli = Cli {
            input_path: test_file,
            output_dir_name: Some("test-output".to_string()),
            no_comments: true,
            no_function_bodies: false,
            no_stats: true,
            dry_run: true,
            single_file: false,
        };

        let processor = create_processor(&cli);
        let result = processor.process_path(&cli.input_path, cli.output_dir_name.as_deref());

        assert!(result.is_ok(), "Processing should succeed");
        Ok(())
    }

    #[test]
    fn test_main_full_workflow() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "fn main() { println!(\"test\"); }")?;

        let args = vec![
            "program",
            test_file.to_str().unwrap(),
            "--no-comments",
            "--dry-run",
        ];
        let cli = Cli::try_parse_from(args)?;

        // Use try_init() instead of init() to handle case where logger is already initialized
        let _ = tracing_subscriber::fmt::try_init();
        let processor = create_processor(&cli);
        let stats = processor.process_path(&cli.input_path, cli.output_dir_name.as_deref())?;

        assert!(stats.files_processed > 0);
        assert!(stats.input_size > 0);
        assert!(stats.output_size > 0);

        Ok(())
    }
}
