use crate::{
    module_path::ModulePath,
    transformer::{CodeTransformer, RustAnalyzer},
};
use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use syn::visit_mut::VisitMut;
use walkdir::WalkDir;

#[derive(Default, Clone)]
pub struct ProcessingStats {
    pub files_processed: usize,
    pub input_size: usize,
    pub output_size: usize,
}

impl ProcessingStats {
    pub fn reduction_percentage(&self) -> f64 {
        if self.input_size == 0 {
            return 0.0;
        }
        ((self.input_size - self.output_size) as f64 / self.input_size as f64) * 100.0
    }
}

pub trait Processor {
    fn dry_run(&self) -> bool;
    fn single_file(&self) -> bool;
    fn no_comments(&self) -> bool;
    fn process_file(&self, input: &Path, output: &Path) -> Result<(usize, usize)>;

    fn process_directory_to_single_file(
        &self,
        input_dir: &Path,
        output_base: &Path,
    ) -> Result<ProcessingStats> {
        let mut total_stats = ProcessingStats::default();
        let mut combined_output = String::new();

        // Collect all Rust files first
        let rust_files: Vec<_> = WalkDir::new(input_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "rs"))
            .collect();

        let pb = ProgressBar::new(rust_files.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} files {msg}")
                .unwrap()
                .progress_chars("##-"),
        );

        for entry in rust_files.iter() {
            let path = entry.path();
            let relative = path
                .strip_prefix(input_dir)
                .context("Failed to strip prefix from path")?;

            let content = std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read file: {}", path.display()))?;
            let input_size = content.len();

            let module_path = ModulePath::new(path);
            if !module_path.is_valid_module() {
                continue;
            }

            let mut analyzer = RustAnalyzer::new(&content)?;
            let mut transformer = CodeTransformer::new(self.no_comments());
            transformer.visit_file_mut(&mut analyzer.ast);

            let processed_content = prettyplease::unparse(&analyzer.ast);
            let output_size = processed_content.len();

            // Add file header and content to combined output
            combined_output.push_str(&format!("\n// File: {}\n\n", relative.display()));
            combined_output.push_str(&processed_content);
            combined_output.push('\n');

            total_stats.files_processed += 1;
            total_stats.input_size += input_size;
            total_stats.output_size += output_size;
            pb.inc(1);
        }

        pb.finish_with_message("Processing complete!");

        if !self.dry_run() {
            let output_file = output_base.join("code_context.rs.txt");
            if let Some(parent) = output_file.parent() {
                std::fs::create_dir_all(parent)
                    .context("Failed to create output directory for code context")?;
            }
            std::fs::write(output_file, combined_output)
                .context("Failed to write code context file")?;
        }

        Ok(total_stats)
    }

    fn get_output_path(input: &Path, output_dir_name: Option<&str>) -> Result<PathBuf> {
        let output_dir_name = output_dir_name.unwrap_or("code-context");

        // Get the parent directory of the input
        let parent = input.parent().context("Failed to get parent directory")?;

        // For input "foo/bar", create "foo/bar-code-context" in the same directory
        // For input "foo/bar.rs", create "foo/code-context/bar.rs.txt"
        if input.is_file() {
            // For input "foo/bar.rs", create "foo/bar.rs.txt"
            let mut output = input.to_path_buf();
            output.set_extension("rs.txt");
            Ok(output)
        } else {
            // For input "foo/bar", create "foo/bar-code-context" if output_dir_name is not provided
            let dir_name = input
                .file_name()
                .context("Failed to get directory name")?
                .to_str()
                .context("Invalid directory name")?;
            Ok(parent.join(format!("{}-{}", dir_name, output_dir_name)))
        }
    }

    fn process_path(&self, input: &Path, output_dir_name: Option<&str>) -> Result<ProcessingStats> {
        let output_base = Self::get_output_path(input, output_dir_name)?;
        let mut stats = ProcessingStats::default();

        if input.is_file() {
            if let Some(parent) = output_base.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let (input_size, output_size) = self.process_file(input, &output_base)?;
            stats.files_processed = 1;
            stats.input_size = input_size;
            stats.output_size = output_size;
        } else {
            std::fs::create_dir_all(&output_base)?;
            let dir_stats = self.process_directory(input, &output_base)?;
            stats = dir_stats;
        }
        Ok(stats)
    }

    fn process_directory(&self, input_dir: &Path, output_base: &Path) -> Result<ProcessingStats> {
        if self.single_file() {
            return self.process_directory_to_single_file(input_dir, output_base);
        }
        // Collect all Rust files first
        let rust_files: Vec<_> = WalkDir::new(input_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file() && e.path().extension().is_some_and(|ext| ext == "rs"))
            .collect();

        let pb = ProgressBar::new(rust_files.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} files {msg}")
                .unwrap()
                .progress_chars("##-"),
        );

        let mut total_stats = ProcessingStats::default();

        // Process files sequentially instead of in parallel
        for entry in rust_files.iter() {
            let path = entry.path();
            let relative = path
                .strip_prefix(input_dir)
                .context("Failed to strip prefix from path")?;
            let mut output_path = output_base.join(relative);
            output_path.set_extension("rs.txt");

            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent).context("Failed to create output directory")?;
            }

            let (input_size, output_size) = self
                .process_file(path, &output_path)
                .with_context(|| format!("Failed to process file: {}", path.display()))?;

            total_stats.files_processed += 1;
            total_stats.input_size += input_size;
            total_stats.output_size += output_size;
            pb.inc(1);
        }

        pb.finish_with_message("Processing complete!");

        Ok(total_stats)
    }
}

pub struct FileProcessor {
    no_comments: bool,
    dry_run: bool,
    single_file: bool,
}

impl FileProcessor {
    pub fn with_options(no_comments: bool, dry_run: bool, single_file: bool) -> Self {
        Self {
            no_comments,
            dry_run,
            single_file,
        }
    }
}

impl Processor for FileProcessor {
    fn dry_run(&self) -> bool {
        self.dry_run
    }

    fn single_file(&self) -> bool {
        self.single_file
    }

    fn no_comments(&self) -> bool {
        self.no_comments
    }

    fn process_file(&self, input: &Path, output: &Path) -> Result<(usize, usize)> {
        let content = std::fs::read_to_string(input).context("Failed to read input file")?;
        let input_size = content.len();

        let module_path = ModulePath::new(input);
        if !module_path.is_valid_module() {
            return Err(anyhow::anyhow!(
                "Not a valid Rust module file: {}",
                input.display()
            ));
        }

        let mut analyzer = RustAnalyzer::new(&content)?;
        let mut transformer = CodeTransformer::new(self.no_comments);

        transformer.visit_file_mut(&mut analyzer.ast);

        let output_content = prettyplease::unparse(&analyzer.ast);
        let output_size = output_content.len();

        if !self.dry_run() {
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).context("Failed to create output directory")?;
            }
            std::fs::write(output, output_content).context("Failed to write output file")?;
        }

        Ok((input_size, output_size))
    }
}
