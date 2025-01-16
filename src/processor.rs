use crate::{
    module_path::ModulePath,
    transformer::{CodeTransformer, RustAnalyzer},
};
use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use syn::visit_mut::VisitMut;
use walkdir::WalkDir;

#[derive(Default, Clone, Debug)]
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
        // Use f64 for calculation to avoid integer overflow
        let input_size = self.input_size as f64;
        let output_size = self.output_size as f64;
        ((input_size - output_size) / input_size) * 100.0
    }
}

pub trait Processor {
    fn dry_run(&self) -> bool;
    fn single_file(&self) -> bool;
    fn no_comments(&self) -> bool;
    fn no_function_body(&self) -> bool;
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
            let mut transformer = CodeTransformer::new(self.no_comments(), self.no_function_body());
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
        // Validate input path
        if input.as_os_str().is_empty() {
            return Err(anyhow::anyhow!("Input path cannot be empty"));
        }

        if input == Path::new("/") {
            return Err(anyhow::anyhow!("Cannot use root directory as input path"));
        }

        let output_dir_name = output_dir_name.unwrap_or("code-context");

        if input.is_file() {
            let parent = input.parent().unwrap_or_else(|| Path::new("."));

            let file_stem = input
                .file_stem()
                .context("Failed to get file stem")?
                .to_str()
                .context("File stem must be valid UTF-8")?;

            Ok(parent.join(format!("{}-{}", file_stem, output_dir_name)))
        } else {
            let parent = input.parent().unwrap_or_else(|| Path::new("."));

            let dir_name = input
                .file_name()
                .context("Failed to get directory name")?
                .to_str()
                .context("Directory name must be valid UTF-8")?;

            Ok(parent.join(format!("{}-{}", dir_name, output_dir_name)))
        }
    }

    fn process_path(&self, input: &Path, output_dir_name: Option<&str>) -> Result<ProcessingStats> {
        // First verify input path exists
        if !input.try_exists()? {
            return Err(anyhow::anyhow!(
                "Input path does not exist: {}",
                input.display()
            ));
        }

        let output_base = Self::get_output_path(input, output_dir_name)?;
        let mut stats = ProcessingStats::default();

        if !self.dry_run() {
            // Always create the output directory, whether it's a file or directory input
            std::fs::create_dir_all(&output_base)?;
        }

        if input.is_file() {
            let output_file = if output_base.is_dir() {
                output_base
                    .join(input.file_name().unwrap())
                    .with_extension("rs.txt")
            } else {
                output_base
            };
            let (input_size, output_size) = self.process_file(input, &output_file)?;
            stats.files_processed = 1;
            stats.input_size = input_size;
            stats.output_size = output_size;
        } else {
            let dir_stats = self.process_directory(input, &output_base)?;
            stats = dir_stats;
        }
        Ok(stats)
    }

    fn process_directory(&self, input_dir: &Path, output_base: &Path) -> Result<ProcessingStats> {
        if self.single_file() {
            return self.process_directory_to_single_file(input_dir, output_base);
        }

        // Verify output_base doesn't exist as a file
        if output_base.exists() && !output_base.is_dir() {
            return Err(anyhow::anyhow!(
                "Failed to create output directory: '{}' exists and is not a directory",
                output_base.display()
            ));
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
    no_function_bodies: bool,
    dry_run: bool,
    single_file: bool,
}

impl FileProcessor {
    pub fn with_options(
        no_comments: bool,
        no_function_bodies: bool,
        dry_run: bool,
        single_file: bool,
    ) -> Self {
        Self {
            no_comments,
            no_function_bodies,
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

    fn no_function_body(&self) -> bool {
        self.no_function_bodies
    }

    fn process_file(&self, input: &Path, output: &Path) -> Result<(usize, usize)> {
        // Verify input file exists before trying to read it
        if !input.try_exists()? {
            return Err(anyhow::anyhow!(
                "Input file does not exist: {}",
                input.display()
            ));
        }

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
        let mut transformer = CodeTransformer::new(self.no_comments(), self.no_function_body());

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::process_code;
    use crate::{create_processor, Cli};
    use anyhow::Result;
    use clap::Parser;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_process_path_with_file() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let test_file = temp_dir.path().join("test.rs");
        fs::write(&test_file, "fn main() {}")?;

        let processor = FileProcessor::with_options(false, false, false, false);
        let stats = processor.process_path(&test_file, Some("output"))?;

        assert_eq!(stats.files_processed, 1);
        assert!(stats.input_size > 0);
        Ok(())
    }

    #[test]
    fn test_process_directory_to_single_file() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let input_dir = temp_dir.path();

        // Create test files
        fs::create_dir_all(input_dir.join("src"))?;
        fs::write(
            input_dir.join("src/main.rs"),
            "fn main() { println!(\"Hello\"); }",
        )?;
        fs::write(
            input_dir.join("src/lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 { a + b }",
        )?;

        let processor = FileProcessor::with_options(false, false, false, true);
        let output_dir = temp_dir.path().join("output");
        let stats = processor.process_directory_to_single_file(input_dir, &output_dir)?;

        assert!(stats.files_processed > 0);
        assert!(stats.input_size > 0);
        assert!(output_dir.join("code_context.rs.txt").exists());

        Ok(())
    }

    #[test]
    fn test_process_directory() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let input_dir = temp_dir.path();

        // Create test files
        fs::create_dir_all(input_dir.join("src"))?;
        fs::write(
            input_dir.join("src/main.rs"),
            "fn main() { println!(\"Hello\"); }",
        )?;
        fs::write(
            input_dir.join("src/lib.rs"),
            "pub fn add(a: i32, b: i32) -> i32 { a + b }",
        )?;

        let processor = FileProcessor::with_options(false, false, false, false);
        let output_dir = temp_dir.path().join("output");
        let stats = processor.process_directory(input_dir, &output_dir)?;

        assert!(stats.files_processed > 0);
        assert!(stats.input_size > 0);
        assert!(output_dir.join("src").join("main.rs.txt").exists());
        assert!(output_dir.join("src").join("lib.rs.txt").exists());

        Ok(())
    }

    #[test]
    fn test_get_output_path() -> Result<()> {
        let temp_dir = TempDir::new()?;

        // Test file input
        let file_input = temp_dir.path().join("test.rs");
        fs::write(&file_input, "fn main() {}")?;

        assert!(file_input.exists(), "Test file should exist");
        assert!(file_input.is_file(), "Test file should be a file");

        let file_output = FileProcessor::get_output_path(&file_input, Some("test-output"))?;
        assert_eq!(file_output.file_name().unwrap(), "test-test-output");

        // Test directory input
        let dir_input = temp_dir.path().join("src");
        fs::create_dir(&dir_input)?;

        let dir_output = FileProcessor::get_output_path(&dir_input, Some("test-output"))?;
        assert_eq!(
            dir_output.file_name().unwrap().to_str().unwrap(),
            "src-test-output"
        );

        Ok(())
    }

    #[test]
    fn test_get_output_path_edge_cases() -> Result<()> {
        // Test with root path
        let root_path = PathBuf::from("/");
        let result = FileProcessor::get_output_path(&root_path, None);
        assert!(result.is_err(), "Root path should be rejected");

        // Test with empty path
        let empty_path = PathBuf::from("");
        let result = FileProcessor::get_output_path(&empty_path, None);
        assert!(result.is_err(), "Empty path should be rejected");

        // Test with just a file name (no parent directory)
        let file_only = PathBuf::from("file.rs");
        fs::write(&file_only, "fn main() {}")?;
        assert!(file_only.exists(), "Test file should exist");
        assert!(file_only.is_file(), "Test file should be a file");

        let result = FileProcessor::get_output_path(&file_only, None);
        fs::remove_file(&file_only)?;

        assert!(!file_only.exists(), "Test file should be deleted");
        assert!(
            result.is_ok(),
            "Path without parent should use current directory"
        );
        assert_eq!(
            result.unwrap(),
            PathBuf::from("file-code-context"),
            "Should use current directory"
        );
        Ok(())
    }

    #[test]
    fn test_invalid_input_path() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let invalid_path = temp_dir.path().join("nonexistent");
        let processor = FileProcessor::with_options(false, true, false, false);

        let result = processor.process_path(&invalid_path, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));

        Ok(())
    }

    #[test]
    fn test_processor_options() {
        let processor = FileProcessor::with_options(true, true, true, true);
        assert!(processor.no_comments());
        assert!(processor.dry_run());
        assert!(processor.single_file());
    }

    #[test]
    fn test_process_directory_empty() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let processor = FileProcessor::with_options(false, false, false, false);
        let stats = processor.process_directory(temp_dir.path(), temp_dir.path())?;
        assert_eq!(stats.files_processed, 0);
        Ok(())
    }

    #[test]
    fn test_get_output_path_errors() -> Result<()> {
        let path = PathBuf::from("/");
        assert!(FileProcessor::get_output_path(&path, None).is_err());
        Ok(())
    }

    #[test]
    fn test_result_string_function() -> Result<()> {
        let input = r#"
            impl MyStruct {
                fn get_result_string(&self) -> Result<String, Error> {
                    Ok("test".to_string())
                }
                
                fn get_result_number(&self) -> Result<i32, Error> {
                    Ok(42)
                }
            }
        "#;
        let expected = r#"impl MyStruct {
    fn get_result_string(&self) -> Result<String, Error> {
        Ok("test".to_string())
    }
    fn get_result_number(&self) -> Result<i32, Error> {}
}"#;
        assert_eq!(process_code(input, false, true)?.trim(), expected.trim());
        Ok(())
    }

    #[test]
    fn test_cow_str_function() -> Result<()> {
        let input = r#"
            use std::borrow::Cow;
            impl MyStruct {
                fn get_cow_str(&self) -> Cow<'static, str> {
                    Cow::Borrowed("test")
                }
            }
        "#;
        let expected = r#"use std::borrow::Cow;
impl MyStruct {
    fn get_cow_str(&self) -> Cow<'static, str> {
        Cow::Borrowed("test")
    }
}"#;
        assert_eq!(process_code(input, false, true)?.trim(), expected.trim());
        Ok(())
    }

    #[test]
    fn test_derived_impl() -> Result<()> {
        let input = r#"
            #[derive(Debug)]
            impl MyStruct {
                fn derived_method(&self) -> String {
                    "test".to_string()
                }
            }
        "#;
        let expected = r#"#[derive(Debug)]
impl MyStruct {
    fn derived_method(&self) -> String {}
}"#;
        assert_eq!(process_code(input, false, true)?.trim(), expected.trim());
        Ok(())
    }

    #[test]
    fn test_main_full_workflow() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let test_file = temp_dir.path().join("test.rs");
        fs::write(
            &test_file,
            r#"fn main() {
            println!("test");
            println!("more code to increase size");
            let x = 42;
            println!("The answer is {}", x);
        }"#,
        )?;

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

    #[test]
    fn test_process_directory_with_nested_modules() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir)?;

        // Create a nested module structure
        fs::write(
            src_dir.join("main.rs"),
            r#"
            mod submod;
            fn main() { println!("main"); }
            "#,
        )?;

        fs::create_dir_all(src_dir.join("submod"))?;
        fs::write(
            src_dir.join("submod/mod.rs"),
            r#"
            pub mod nested;
            pub fn submod_func() { println!("submod"); }
            "#,
        )?;

        fs::write(
            src_dir.join("submod/nested.rs"),
            r#"
            pub fn nested_func() { println!("nested"); }
            "#,
        )?;

        let processor = FileProcessor::with_options(false, false, false, false);
        let output_dir = temp_dir.path().join("output");
        let stats = processor.process_directory(&src_dir, &output_dir)?;

        assert_eq!(stats.files_processed, 3);
        assert!(stats.input_size > 0);
        assert!(stats.output_size > 0);

        // Verify output structure
        assert!(output_dir.join("main.rs.txt").exists());
        assert!(output_dir.join("submod/mod.rs.txt").exists());
        assert!(output_dir.join("submod/nested.rs.txt").exists());

        Ok(())
    }

    #[test]
    fn test_process_directory_with_test_modules() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir)?;

        // Create a module with test code
        fs::write(
            src_dir.join("lib.rs"),
            r#"
            pub fn add(a: i32, b: i32) -> i32 { a + b }

            #[cfg(test)]
            mod tests {
                use super::*;
                
                #[test]
                fn test_add() {
                    assert_eq!(add(2, 2), 4);
                }
            }
            "#,
        )?;

        let processor = FileProcessor::with_options(false, false, false, false);
        let output_dir = temp_dir.path().join("output");
        let stats = processor.process_directory(&src_dir, &output_dir)?;

        assert_eq!(stats.files_processed, 1);

        // Verify test module was removed
        let output_content = fs::read_to_string(output_dir.join("lib.rs.txt"))?;
        assert!(!output_content.contains("#[cfg(test)]"));
        assert!(!output_content.contains("mod tests"));
        assert!(!output_content.contains("test_add"));

        Ok(())
    }

    #[test]
    fn test_process_directory_with_comments() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir)?;

        fs::write(
            src_dir.join("lib.rs"),
            r#"
            //! Module documentation
            
            /// Function documentation
            pub fn documented_function() -> String {
                "Hello".to_string()
            }
            "#,
        )?;

        // Test with comments preserved
        let processor = FileProcessor::with_options(false, false, false, false);
        let output_dir = temp_dir.path().join("output-with-comments");
        processor.process_directory(&src_dir, &output_dir)?;

        let content = fs::read_to_string(output_dir.join("lib.rs.txt"))?;
        assert!(content.contains("//! Module documentation"));
        assert!(content.contains("/// Function documentation"));

        // Test with comments removed
        let processor = FileProcessor::with_options(true, false, false, false);
        let output_dir = temp_dir.path().join("output-no-comments");
        processor.process_directory(&src_dir, &output_dir)?;

        let content = fs::read_to_string(output_dir.join("lib.rs.txt"))?;
        assert!(!content.contains("//! Module documentation"));
        assert!(!content.contains("/// Function documentation"));

        Ok(())
    }

    #[test]
    fn test_process_directory_with_single_file_output() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir)?;

        // Create multiple source files
        fs::write(
            src_dir.join("main.rs"),
            r#"fn main() {
                println!("Hello");
                println!("This is a much longer function");
                let x = 42;
                let y = x * 2;
                println!("The answer is: {}", x);
                println!("Double the answer is: {}", y);
            }"#,
        )?;
        fs::write(
            src_dir.join("lib.rs"),
            r#"pub fn lib_function() { println!("lib"); }"#,
        )?;

        let processor = FileProcessor::with_options(false, false, false, false);
        let output_dir = temp_dir.path().join("output");
        let stats = processor.process_directory_to_single_file(&src_dir, &output_dir)?;

        assert_eq!(stats.files_processed, 2);
        assert!(output_dir.join("code_context.rs.txt").exists());

        // Verify content includes both files
        let content = fs::read_to_string(output_dir.join("code_context.rs.txt"))?;
        assert!(content.contains("// File: main.rs"));
        assert!(content.contains("// File: lib.rs"));

        Ok(())
    }

    #[test]
    fn test_process_path_with_nonexistent_parent() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let nonexistent_parent = temp_dir.path().join("nonexistent").join("test.rs");

        let processor = FileProcessor::with_options(false, false, false, false);
        let result = processor.process_path(&nonexistent_parent, None);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
        Ok(())
    }

    #[test]
    fn test_process_directory_with_invalid_files() -> Result<()> {
        let temp_dir = TempDir::new()?;

        // Create some invalid files
        fs::write(temp_dir.path().join("test.txt"), "not rust")?;
        fs::write(temp_dir.path().join("test.rs.txt"), "not rust module")?;

        let processor = FileProcessor::with_options(false, false, false, false);
        let stats = processor.process_directory(temp_dir.path(), temp_dir.path())?;

        // Should skip non-rust and .rs.txt files
        assert_eq!(stats.files_processed, 0);
        Ok(())
    }

    #[test]
    fn test_process_directory_with_unreadable_file() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let rust_file = temp_dir.path().join("test.rs");
        fs::write(&rust_file, "invalid rust code @#$%")?;

        let processor = FileProcessor::with_options(false, false, false, false);
        let result = processor.process_directory(temp_dir.path(), temp_dir.path());

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to process file"));
        Ok(())
    }

    #[test]
    fn test_process_directory_with_output_creation_error() -> Result<()> {
        let temp_dir = TempDir::new()?;

        // Create a valid Rust source file
        let src_dir = temp_dir.path().join("src");
        fs::create_dir(&src_dir)?;
        let input_file = src_dir.join("test.rs");
        fs::write(&input_file, "fn main() {}")?;

        // Create a regular file at the intended output directory path
        let output_path = temp_dir.path().join("output");
        fs::write(&output_path, "blocking file")?;

        let processor = FileProcessor::with_options(false, false, false, false);
        let result = processor.process_directory(&src_dir, &output_path);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to create output directory"));
        Ok(())
    }

    #[test]
    fn test_process_directory_to_single_file_empty() -> Result<()> {
        let temp_dir = TempDir::new()?;

        let processor = FileProcessor::with_options(false, false, false, false);
        let stats = processor.process_directory_to_single_file(temp_dir.path(), temp_dir.path())?;

        assert_eq!(stats.files_processed, 0);
        assert_eq!(stats.input_size, 0);
        assert_eq!(stats.output_size, 0);
        Ok(())
    }

    #[test]
    fn test_process_directory_to_single_file_with_invalid_content() -> Result<()> {
        let temp_dir = TempDir::new()?;
        fs::write(temp_dir.path().join("test.rs"), "invalid rust @#$%")?;

        let processor = FileProcessor::with_options(false, false, true, false);
        let result = processor.process_directory_to_single_file(temp_dir.path(), temp_dir.path());

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse Rust file"));
        Ok(())
    }

    #[test]
    fn test_process_file_with_errors() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let input_file = temp_dir.path().join("test.rs");
        fs::write(&input_file, "fn main() {}")?;

        // Create a directory with the same name as our target output file
        // This will cause a write error since we can't write a file over a directory
        let output_file = temp_dir.path().join("output");
        fs::create_dir(&output_file)?;

        let processor = FileProcessor::with_options(false, false, false, false);
        let result = processor.process_file(&input_file, &output_file);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to write"));

        // Test an error with wrong input file
        let invalid_file = PathBuf::from("/invalid/file.rs");
        let result = processor.process_file(&invalid_file, &output_file);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
        Ok(())
    }

    #[test]
    fn test_processing_stats() {
        let mut stats = ProcessingStats::default();
        assert_eq!(stats.files_processed, 0);
        assert_eq!(stats.input_size, 0);
        assert_eq!(stats.output_size, 0);
        assert_eq!(stats.reduction_percentage(), 0.0);

        stats.files_processed = 1;
        stats.input_size = 100;
        stats.output_size = 50;
        assert_eq!(stats.reduction_percentage(), 50.0);

        stats.input_size = 200;
        stats.output_size = 100;
        assert_eq!(stats.reduction_percentage(), 50.0);

        stats.input_size = 0;
        assert_eq!(stats.reduction_percentage(), 0.0);
    }

    #[test]
    fn test_processing_stats_methods() {
        let stats = ProcessingStats {
            files_processed: 0,
            input_size: 100,
            output_size: 0,
        };
        assert_eq!(stats.reduction_percentage(), 100.0);

        let stats = ProcessingStats {
            files_processed: 0,
            input_size: 0,
            output_size: 0,
        };
        assert_eq!(stats.reduction_percentage(), 0.0);
    }

    #[test]
    fn test_processing_stats_clone() {
        let stats = ProcessingStats {
            files_processed: 5,
            input_size: 1000,
            output_size: 500,
        };
        let cloned = stats.clone();
        assert_eq!(stats.files_processed, cloned.files_processed);
        assert_eq!(stats.input_size, cloned.input_size);
        assert_eq!(stats.output_size, cloned.output_size);
        assert_eq!(stats.reduction_percentage(), cloned.reduction_percentage());
    }

    #[test]
    fn test_processing_stats_debug() {
        let stats = ProcessingStats {
            files_processed: 3,
            input_size: 150,
            output_size: 75,
        };
        let debug_str = format!("{:?}", stats);
        assert!(debug_str.contains("files_processed: 3"));
        assert!(debug_str.contains("input_size: 150"));
        assert!(debug_str.contains("output_size: 75"));
    }

    #[test]
    fn test_processing_stats_edge_cases() {
        let stats = ProcessingStats {
            files_processed: 0,
            input_size: 0,
            output_size: 0,
        };
        assert_eq!(stats.reduction_percentage(), 0.0);

        let stats = ProcessingStats {
            files_processed: 1,
            input_size: 100,
            output_size: 0,
        };
        assert_eq!(stats.reduction_percentage(), 100.0);

        let stats = ProcessingStats {
            files_processed: 1,
            input_size: 100,
            output_size: 100,
        };
        assert_eq!(stats.reduction_percentage(), 0.0);

        let stats = ProcessingStats {
            files_processed: 1,
            input_size: 100,
            output_size: 200, // Output larger than input
        };
        assert_eq!(stats.reduction_percentage(), -100.0);
    }

    #[test]
    fn test_processing_stats_accumulation() {
        let mut total_stats = ProcessingStats::default();

        // Simulate processing multiple files
        let file1_stats = ProcessingStats {
            files_processed: 1,
            input_size: 100,
            output_size: 50,
        };

        let file2_stats = ProcessingStats {
            files_processed: 1,
            input_size: 200,
            output_size: 100,
        };

        total_stats.files_processed += file1_stats.files_processed + file2_stats.files_processed;
        total_stats.input_size += file1_stats.input_size + file2_stats.input_size;
        total_stats.output_size += file1_stats.output_size + file2_stats.output_size;

        assert_eq!(total_stats.files_processed, 2);
        assert_eq!(total_stats.input_size, 300);
        assert_eq!(total_stats.output_size, 150);
        assert_eq!(total_stats.reduction_percentage(), 50.0);
    }

    #[test]
    fn test_processing_stats_large_numbers() {
        let stats = ProcessingStats {
            files_processed: usize::MAX,
            input_size: usize::MAX,
            output_size: usize::MAX / 2,
        };
        assert_eq!(stats.reduction_percentage(), 50.0);

        let stats = ProcessingStats {
            files_processed: usize::MAX,
            input_size: usize::MAX,
            output_size: 0,
        };
        assert_eq!(stats.reduction_percentage(), 100.0);
    }
}
